//! One method per LeafWiki route burrow uses (v0.14). Pages and tree nodes are typed because
//! the tools reason about them; other answers stay JSON and pass through in LeafWiki's order.

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value, json};

use crate::http::{Body, Method, Res, Session, Spec, WikiErr};

/// `encodeURIComponent`: everything but letters, digits and `-_.!~*'()`.
const COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'!')
    .remove(b'~')
    .remove(b'*')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')');

fn enc(segment: &str) -> String {
    utf8_percent_encode(segment, COMPONENT).to_string()
}

/// Page paths are stored without leading or trailing slashes.
pub fn normalize_path(path: &str) -> &str {
    path.trim().trim_matches('/')
}

/// LeafWiki sends `null` for empty lists and missing values; both read as the default here.
fn nd<'de, D: Deserializer<'de>, T: Deserialize<'de> + Default>(d: D) -> Result<T, D::Error> {
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct UserRef {
    #[serde(default)]
    pub username: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Metadata {
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub creator: Option<UserRef>,
    #[serde(default)]
    pub last_author: Option<UserRef>,
}

/// A page, or a tree node (the same shape without content, tags and properties).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Page {
    #[serde(default, deserialize_with = "nd")]
    pub id: String,
    #[serde(default, deserialize_with = "nd")]
    pub title: String,
    #[serde(default, deserialize_with = "nd")]
    pub slug: String,
    #[serde(default, deserialize_with = "nd")]
    pub path: String,
    /// Optimistic-concurrency token: send it back with every change.
    #[serde(default, deserialize_with = "nd")]
    pub version: String,
    #[serde(default, deserialize_with = "nd")]
    pub kind: String,
    #[serde(default, deserialize_with = "nd")]
    pub pinned: bool,
    #[serde(default, deserialize_with = "nd")]
    pub children: Vec<Page>,
    #[serde(default, deserialize_with = "nd")]
    pub metadata: Metadata,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub properties: Option<Map<String, Value>>,
}

impl Page {
    pub fn text(&self) -> &str {
        self.content.as_deref().unwrap_or("")
    }
}

/// What a full save sends: LeafWiki's `PUT /pages/:id` replaces tags and properties.
pub struct PageUpdate {
    pub version: String,
    pub title: String,
    pub slug: String,
    pub content: Option<String>,
    pub tags: Vec<String>,
    pub properties: Map<String, Value>,
}

pub struct Wiki {
    pub session: Session,
}

fn typed<T: for<'de> Deserialize<'de>>(r: Res<Value>) -> Res<T> {
    let value = r?;
    T::deserialize(value).map_err(|e| WikiErr {
        status: 200,
        message: format!("LeafWiki answered with an unexpected shape: {e}"),
        code: None,
        body: String::new(),
        request: None,
    })
}

impl Wiki {
    async fn get(&self, spec: Spec) -> Res<Value> {
        self.session.request(spec).await
    }

    pub async fn config(&self) -> Res<Value> {
        self.get(Spec::new(Method::Get, "/config")).await
    }
    pub async fn me(&self) -> Res<Value> {
        self.get(Spec::new(Method::Get, "/auth/me")).await
    }
    pub async fn tree(&self) -> Res<Page> {
        typed(self.get(Spec::new(Method::Get, "/tree")).await)
    }
    pub async fn page(&self, id: &str) -> Res<Page> {
        typed(
            self.get(Spec::new(Method::Get, format!("/pages/{}", enc(id))))
                .await,
        )
    }
    pub async fn page_by_path(&self, path: &str) -> Res<Page> {
        typed(
            self.get(
                Spec::new(Method::Get, "/pages/by-path").query("path", Some(normalize_path(path))),
            )
            .await,
        )
    }
    pub async fn lookup(&self, path: &str) -> Res<Value> {
        self.get(Spec::new(Method::Get, "/pages/lookup").query("path", Some(normalize_path(path))))
            .await
    }
    pub async fn by_title(&self, title: &str) -> Res<Value> {
        self.get(Spec::new(Method::Get, "/pages/by-title").query("title", Some(title)))
            .await
    }
    pub async fn slug(
        &self,
        title: &str,
        parent_id: Option<&str>,
        current_id: Option<&str>,
    ) -> Res<Value> {
        let spec = Spec::new(Method::Get, "/pages/slug-suggestion")
            .query("title", Some(title))
            .query("parentId", parent_id)
            .query("currentId", current_id);
        self.get(spec).await
    }

    /// A page by id or by path. A value with a `/` is a path; otherwise the id is tried first
    /// and the path second, since top-level slugs look like ids.
    pub async fn resolve(&self, reference: &str) -> Res<Page> {
        let value = reference.trim();
        if value.contains('/') {
            return self.page_by_path(value).await;
        }
        let by_id = self.page(value).await;
        match &by_id {
            Err(e) if e.status == 404 => {}
            _ => return by_id,
        }
        self.page_by_path(value).await.map_err(|e| WikiErr {
            message: format!("No page has the id or path \"{value}\""),
            ..e
        })
    }

    pub async fn search(&self, q: &str, offset: Option<i64>, limit: Option<i64>) -> Res<Value> {
        let spec = Spec::new(Method::Get, "/search")
            .query("q", Some(q))
            .query("offset", offset)
            .query("limit", limit);
        self.get(spec).await
    }
    pub async fn indexing(&self) -> Res<Value> {
        self.get(Spec::new(Method::Get, "/search/status")).await
    }
    pub async fn tags(&self, q: Option<&str>, limit: Option<i64>) -> Res<Value> {
        self.get(
            Spec::new(Method::Get, "/tags")
                .query("q", q)
                .query("limit", limit),
        )
        .await
    }
    pub async fn pages_by_tags(&self, tags: &[String]) -> Res<Value> {
        let mut spec = Spec::new(Method::Get, "/tags/pages");
        for tag in tags {
            spec = spec.query("tags", Some(tag));
        }
        self.get(spec).await
    }
    pub async fn property_keys(&self, q: Option<&str>, limit: Option<i64>) -> Res<Value> {
        self.get(
            Spec::new(Method::Get, "/properties")
                .query("q", q)
                .query("limit", limit),
        )
        .await
    }
    pub async fn pages_by_property(&self, key: &str, value: Option<&str>) -> Res<Value> {
        self.get(
            Spec::new(Method::Get, "/properties/pages")
                .query("key", Some(key))
                .query("value", value),
        )
        .await
    }
    pub async fn links(&self, id: &str) -> Res<Value> {
        self.get(Spec::new(Method::Get, format!("/pages/{}/links", enc(id))))
            .await
    }
    pub async fn broken_links(&self) -> Res<Value> {
        self.get(Spec::new(Method::Get, "/links/broken")).await
    }

    pub async fn revisions(
        &self,
        id: &str,
        limit: Option<i64>,
        cursor: Option<&str>,
    ) -> Res<Value> {
        let spec = Spec::new(Method::Get, format!("/pages/{}/revisions", enc(id)))
            .query("limit", limit)
            .query("cursor", cursor);
        self.get(spec).await
    }
    pub async fn latest_revision(&self, id: &str) -> Res<Value> {
        self.get(Spec::new(
            Method::Get,
            format!("/pages/{}/revisions/latest", enc(id)),
        ))
        .await
    }
    pub async fn revision(&self, id: &str, revision: &str) -> Res<Value> {
        self.get(Spec::new(
            Method::Get,
            format!("/pages/{}/revisions/{}", enc(id), enc(revision)),
        ))
        .await
    }
    pub async fn compare(&self, id: &str, base: &str, target: &str) -> Res<Value> {
        let spec = Spec::new(Method::Get, format!("/pages/{}/revisions/compare", enc(id)))
            .query("base", Some(base))
            .query("target", Some(target));
        self.get(spec).await
    }
    pub async fn restore(&self, id: &str, revision: &str) -> Res<Page> {
        typed(
            self.get(Spec::new(
                Method::Post,
                format!("/pages/{}/revisions/{}/restore", enc(id), enc(revision)),
            ))
            .await,
        )
    }

    pub async fn favorites(&self) -> Res<Value> {
        self.get(Spec::new(Method::Get, "/favorites")).await
    }
    pub async fn favorite(&self, id: &str, on: bool) -> Res<Value> {
        let method = if on { Method::Put } else { Method::Delete };
        self.get(Spec::new(method, format!("/pages/{}/favorite", enc(id))))
            .await
    }

    pub async fn create(
        &self,
        parent_id: Option<&str>,
        title: &str,
        slug: &str,
        kind: &str,
    ) -> Res<Page> {
        let mut body = Map::new();
        if let Some(parent) = parent_id {
            body.insert("parentId".into(), json!(parent));
        }
        body.insert("title".into(), json!(title));
        body.insert("slug".into(), json!(slug));
        body.insert("kind".into(), json!(kind));
        typed(
            self.get(Spec::new(Method::Post, "/pages").json(Value::Object(body)))
                .await,
        )
    }
    pub async fn ensure(&self, path: &str, title: &str, kind: &str) -> Res<Page> {
        let body = json!({ "path": normalize_path(path), "title": title, "kind": kind });
        typed(
            self.get(Spec::new(Method::Post, "/pages/ensure").json(body))
                .await,
        )
    }
    pub async fn update(&self, id: &str, input: PageUpdate) -> Res<Page> {
        let mut body = Map::new();
        body.insert("version".into(), json!(input.version));
        body.insert("title".into(), json!(input.title));
        body.insert("slug".into(), json!(input.slug));
        if let Some(content) = input.content {
            body.insert("content".into(), json!(content));
        }
        body.insert("tags".into(), json!(input.tags));
        body.insert("properties".into(), Value::Object(input.properties));
        typed(
            self.get(
                Spec::new(Method::Put, format!("/pages/{}", enc(id))).json(Value::Object(body)),
            )
            .await,
        )
    }
    pub async fn remove(&self, id: &str, version: &str, recursive: bool) -> Res<Value> {
        let spec = Spec::new(Method::Delete, format!("/pages/{}", enc(id)))
            .query("version", Some(version))
            .query("recursive", Some(recursive));
        self.get(spec).await
    }
    pub async fn move_page(
        &self,
        id: &str,
        version: &str,
        parent_id: &str,
        position: Option<i64>,
    ) -> Res<Value> {
        let mut body = Map::new();
        body.insert("version".into(), json!(version));
        body.insert("parentId".into(), json!(parent_id));
        if let Some(p) = position {
            body.insert("position".into(), json!(p));
        }
        self.get(
            Spec::new(Method::Put, format!("/pages/{}/move", enc(id))).json(Value::Object(body)),
        )
        .await
    }
    pub async fn sort(&self, parent_id: &str, ordered_ids: &[String]) -> Res<Value> {
        self.get(
            Spec::new(Method::Put, format!("/pages/{}/sort", enc(parent_id)))
                .json(json!({ "orderedIds": ordered_ids })),
        )
        .await
    }
    pub async fn pin(&self, id: &str, version: &str, pinned: bool) -> Res<Page> {
        let body = json!({ "version": version, "pinned": pinned });
        typed(
            self.get(Spec::new(Method::Put, format!("/pages/{}/pin", enc(id))).json(body))
                .await,
        )
    }
    pub async fn convert(&self, id: &str, version: &str, kind: &str) -> Res<Value> {
        let body = json!({ "targetKind": kind, "version": version });
        self.get(Spec::new(Method::Post, format!("/pages/convert/{}", enc(id))).json(body))
            .await
    }
    pub async fn copy(
        &self,
        id: &str,
        target_parent_id: Option<&str>,
        title: &str,
        slug: &str,
    ) -> Res<Page> {
        let body = json!({ "targetParentId": target_parent_id, "title": title, "slug": slug });
        typed(
            self.get(Spec::new(Method::Post, format!("/pages/copy/{}", enc(id))).json(body))
                .await,
        )
    }
    pub async fn refactor_preview(&self, id: &str, input: Value) -> Res<Value> {
        self.get(
            Spec::new(Method::Post, format!("/pages/{}/refactor/preview", enc(id))).json(input),
        )
        .await
    }
    pub async fn refactor_apply(&self, id: &str, input: Value) -> Res<Page> {
        typed(
            self.get(
                Spec::new(Method::Post, format!("/pages/{}/refactor/apply", enc(id))).json(input),
            )
            .await,
        )
    }

    pub async fn assets(&self, id: &str) -> Res<Value> {
        self.get(Spec::new(Method::Get, format!("/pages/{}/assets", enc(id))))
            .await
    }
    pub async fn upload(&self, id: &str, filename: &str, bytes: Vec<u8>) -> Res<Value> {
        let mut spec = Spec::new(Method::Post, format!("/pages/{}/assets", enc(id)));
        spec.body = Body::File {
            name: filename.to_string(),
            bytes,
        };
        self.get(spec).await
    }
    pub async fn rename_asset(&self, id: &str, from: &str, to: &str) -> Res<Value> {
        let body = json!({ "old_filename": from, "new_filename": to });
        self.get(Spec::new(Method::Put, format!("/pages/{}/assets/rename", enc(id))).json(body))
            .await
    }
    pub async fn delete_asset(&self, id: &str, filename: &str) -> Res<Value> {
        self.get(Spec::new(
            Method::Delete,
            format!("/pages/{}/assets/{}", enc(id), enc(filename)),
        ))
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_path_segments_like_encode_uri_component() {
        assert_eq!(enc("a b/c?d#é~*'()!"), "a%20b%2Fc%3Fd%23%C3%A9~*'()!");
        assert_eq!(normalize_path(" /docs/team/ "), "docs/team");
    }

    #[test]
    fn reads_nulls_as_empty() {
        let page: Page = serde_json::from_value(
            json!({ "id": "p", "children": null, "tags": null, "pinned": null, "metadata": null }),
        )
        .unwrap();
        assert!(page.children.is_empty() && page.tags.is_none() && !page.pinned);
    }
}
