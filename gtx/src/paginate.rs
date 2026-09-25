use std::future::Future;

use eyre::Result;

/// Fetch multiple pages until `limit` items are collected or no more results.
///
/// `fetch` receives (page, per_page) and returns a batch. Pagination stops
/// when a batch returns fewer items than requested or `limit` is reached.
pub async fn paginate<T, F, Fut>(limit: i64, per_page: i64, fetch: F) -> Result<Vec<T>>
where
    F: Fn(i64, i64) -> Fut,
    Fut: Future<Output = Result<Vec<T>>>,
{
    paginate_filtered(limit, per_page.min(limit), fetch, |_| true).await
}

/// Like [`paginate`], but keeps only the items `keep` accepts, fetching
/// further pages of `per_page` until `limit` of them are collected.
pub async fn paginate_filtered<T, F, Fut>(
    limit: i64,
    per_page: i64,
    fetch: F,
    keep: impl Fn(&T) -> bool,
) -> Result<Vec<T>>
where
    F: Fn(i64, i64) -> Fut,
    Fut: Future<Output = Result<Vec<T>>>,
{
    let mut all = Vec::new();
    // Every page must be the same size: Gitea's offset is (page - 1) * limit.
    let per_page = per_page.max(1);
    let mut page = 1;
    while (all.len() as i64) < limit {
        let batch = fetch(page, per_page).await?;
        let got = batch.len() as i64;
        all.extend(batch.into_iter().filter(|t| keep(t)));
        if got < per_page {
            break;
        }
        page += 1;
    }
    all.truncate(limit.max(0) as usize);
    Ok(all)
}
