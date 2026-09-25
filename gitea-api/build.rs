use std::{
    env,
    fs::{self, File},
    path::Path,
};

use serde_json::{Value, json};

/// Operations whose documented success body the server never sends: Gitea
/// answers 204 No Content, so drop the body from the spec.
const BODYLESS_DESPITE_SPEC: &[&str] = &["deleteRepoVariable", "deleteOrgVariable"];

/// Gitea's swagger under-documents success codes. E.g. `PUT
/// .../actions/secrets/{name}` lists only 201 (created), but the server
/// answers 204 when updating an existing secret; `POST .../tags` lists 200
/// but the server answers 201. Progenitor maps any undocumented status to
/// `Error::UnexpectedResponse`, so a successful call surfaced as e.g.
/// "HTTP 204: No Content" (#5). Wherever every documented 2xx response has
/// the same shape, replace them with a single `2XX` of that shape so any
/// success status is accepted.
fn accept_any_2xx(spec: &mut Value) {
    let shared = spec["components"]["responses"].clone();
    let resolve = |resp: &Value| -> Value {
        match resp["$ref"].as_str() {
            Some(r) => shared[r.rsplit('/').next().unwrap_or_default()].clone(),
            None => resp.clone(),
        }
    };
    let body = |resp: &Value| -> Value {
        match &resolve(resp)["content"] {
            Value::Object(c) if !c.is_empty() => Value::Object(c.clone()),
            _ => Value::Null,
        }
    };

    let Some(paths) = spec["paths"].as_object_mut() else {
        return;
    };
    for op in paths
        .values_mut()
        .filter_map(Value::as_object_mut)
        .flat_map(|methods| methods.values_mut())
    {
        let bodyless = op["operationId"]
            .as_str()
            .is_some_and(|id| BODYLESS_DESPITE_SPEC.contains(&id));
        let Some(responses) = op.get_mut("responses").and_then(Value::as_object_mut) else {
            continue;
        };
        let success: Vec<String> = responses
            .keys()
            .filter(|code| code.starts_with('2'))
            .cloned()
            .collect();
        let Some(first) = success.first() else {
            continue;
        };
        let shape = body(&responses[first]);
        if !bodyless && success.iter().any(|code| body(&responses[code]) != shape) {
            continue;
        }
        let merged = if bodyless || shape.is_null() {
            json!({ "description": "success" })
        } else {
            responses[first].clone()
        };
        // Replace rather than add: a 2XX arm alongside the exact codes would
        // make those arms unreachable in the generated match.
        for code in &success {
            responses.remove(code);
        }
        responses.insert("2XX".into(), merged);
    }
}

/// Fields the server sends that its swagger omits: Gitea's
/// `ActionWorkflowRun` has `created_at`/`updated_at` (#30) and, for
/// pull_request runs, `pull_requests` (#31): a slim `{id, number, url, head,
/// base}` per PR, where `head`/`base` are `{ref, sha, repo}` with a
/// `{id, url, name}` repo, so not the spec's full `PullRequest`.
fn add_missing_fields(spec: &mut Value) {
    let schemas = &mut spec["components"]["schemas"];
    let branch = json!({ "type": "string" });
    schemas["ActionWorkflowRunPullRequestBranch"] = json!({
        "type": "object",
        "properties": { "ref": branch, "sha": { "type": "string" } },
    });
    let pr_branch = json!({ "$ref": "#/components/schemas/ActionWorkflowRunPullRequestBranch" });
    schemas["ActionWorkflowRunPullRequest"] = json!({
        "type": "object",
        "properties": {
            "id": { "type": "integer", "format": "int64" },
            "number": { "type": "integer", "format": "int64" },
            "url": { "type": "string" },
            "head": pr_branch,
            "base": pr_branch,
        },
    });

    let time = json!({ "type": "string", "format": "date-time" });
    let missing: &[(&str, &[(&str, Value)])] = &[(
        "ActionWorkflowRun",
        &[
            ("created_at", time.clone()),
            ("updated_at", time),
            (
                "pull_requests",
                json!({
                    "type": "array",
                    "items": { "$ref": "#/components/schemas/ActionWorkflowRunPullRequest" },
                }),
            ),
        ],
    )];
    for (schema, fields) in missing {
        let props = &mut schemas[schema]["properties"];
        assert!(props.is_object(), "schema {schema} has no properties");
        for (field, ty) in *fields {
            if props.get(field).is_none() {
                props[field] = ty.clone();
            }
        }
    }
}

fn main() {
    let src = "openapi.v1.json";
    println!("cargo:rerun-if-changed={src}");

    let file = File::open(src).unwrap();
    let mut spec: Value = serde_json::from_reader(file).unwrap();
    accept_any_2xx(&mut spec);
    add_missing_fields(&mut spec);
    let spec = serde_json::from_value(spec).unwrap();

    let mut settings = progenitor::GenerationSettings::new();
    settings
        .with_interface(progenitor::InterfaceStyle::Builder)
        .with_tag(progenitor::TagStyle::Merged);

    let mut generator = progenitor::Generator::new(&settings);
    let tokens = generator.generate_tokens(&spec).unwrap();
    let ast = syn::parse2(tokens).unwrap();
    let mut content = prettyplease::unparse(&ast);

    content = content.replace(
        r#"#[serde(default, skip_serializing_if = "::std::vec::Vec::is_empty")]"#,
        r#"#[serde(default, deserialize_with = "crate::null_as_default", skip_serializing_if = "::std::vec::Vec::is_empty")]"#,
    );

    // For error statuses the spec documents, progenitor emits
    // `Error::ErrorResponse(ResponseValue::empty(response))`, which keeps only
    // the status and headers and drops the response, and with it the body
    // the `exec` hook captured (see `verbose.rs`). Hand the whole response
    // back instead, as for undocumented statuses, so `GiteaError::from` can
    // show the server's reason rather than just "HTTP 422: Unprocessable
    // Entity" (#7).
    const EMPTY_ERROR: &str = "Err(Error::ErrorResponse(ResponseValue::empty(response)))";
    assert!(
        content.contains(EMPTY_ERROR),
        "progenitor's error arm changed shape"
    );
    content = content.replace(EMPTY_ERROR, "Err(Error::UnexpectedResponse(response))");

    let out_file = Path::new(&env::var("OUT_DIR").unwrap()).join("codegen.rs");
    fs::write(out_file, content).unwrap();
}
