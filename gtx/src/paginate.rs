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
    let mut all = Vec::new();
    let mut page = 1;
    loop {
        let remaining = limit - all.len() as i64;
        if remaining <= 0 {
            break;
        }
        let want = remaining.min(per_page);
        let batch = fetch(page, want).await?;
        let got = batch.len() as i64;
        all.extend(batch);
        if got < want {
            break;
        }
        page += 1;
    }
    all.truncate(limit as usize);
    Ok(all)
}
