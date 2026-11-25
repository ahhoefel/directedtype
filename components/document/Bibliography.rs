// DirectedType Standard Component Library: Bibliography Companion
// Parses inline BibTeX `data:` or sandboxed `src:` assets via `FileResolver`, collects `\Cite` nodes
// in pre-order, sorts cited entries (`"citation"` or `"alpha"`), and rewrites citations into `\Link(to:)`.

use directedtype::prelude::*;
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Debug)]
struct BibRecord {
    _key: String,
    author: String,
    title: String,
    venue: String,
    year: String,
}

#[derive(Default, Debug, Clone)]
pub struct Bibliography;

impl Bibliography {
    pub fn new() -> Self {
        Self
    }
}

#[component]
impl Bibliography {
    pub fn on_prepare(&mut self, ctx: &mut PrepareContext<'_>) -> Result<(), DispatchError> {
        let bib_handle = ctx.self_handle();
        let sort_mode = ctx
            .get_port_literal(bib_handle, "sort")?
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_else(|| "citation".to_string());

        let raw_bib = if let Some(inline) = ctx
            .get_port_literal(bib_handle, "data")?
            .and_then(|v| v.as_str().map(str::to_string))
            .filter(|s| !s.is_empty())
        {
            inline
        } else if let Some(src_path) = ctx
            .get_port_literal(bib_handle, "src")?
            .and_then(|v| v.as_str().map(str::to_string))
            .filter(|s| !s.is_empty())
        {
            let bytes = ctx.read_asset(&src_path)?;
            String::from_utf8(bytes).map_err(|e| {
                DispatchError::Custom(format!("Invalid UTF-8 in '{}': {}", src_path, e))
            })?
        } else {
            return Err(DispatchError::Custom(
                "\\Bibliography requires either a `src:` asset path or inline `data:` literal"
                    .into(),
            ));
        };

        let records = parse_bibtex_records(&raw_bib);

        let cite_nodes = ctx.query_by_tag(None, &["Cite"])?;
        let mut first_seen_order: Vec<String> = Vec::new();
        let mut cite_key_by_node: Vec<(GuestNodeHandle, String)> = Vec::new();

        for cite in cite_nodes {
            let key = ctx
                .get_port_literal(cite, "key")?
                .and_then(|v| v.as_str().map(str::to_string))
                .ok_or_else(|| {
                    DispatchError::Custom("\\Cite requires a literal `key:` port".into())
                })?;
            if !first_seen_order.contains(&key) {
                first_seen_order.push(key.clone());
            }
            cite_key_by_node.push((cite, key));
        }

        let mut ordered_keys = first_seen_order;
        if sort_mode == "alpha" {
            ordered_keys.sort_by(|a, b| {
                let ra = records.get(a);
                let rb = records.get(b);
                let ka = (
                    ra.map(|r| r.author.as_str()).unwrap_or(a.as_str()),
                    ra.map(|r| r.year.as_str()).unwrap_or(""),
                    a.as_str(),
                );
                let kb = (
                    rb.map(|r| r.author.as_str()).unwrap_or(b.as_str()),
                    rb.map(|r| r.year.as_str()).unwrap_or(""),
                    b.as_str(),
                );
                ka.cmp(&kb)
            });
        }

        let mut key_to_num_and_anchor: HashMap<String, (usize, GuestNodeHandle)> = HashMap::new();
        for (idx, key) in ordered_keys.iter().enumerate() {
            let num = idx + 1;
            let rec = records.get(key).ok_or_else(|| {
                DispatchError::Custom(format!("Undefined citation key '{}'", key))
            })?;

            let anchor = ctx.create_element(
                "Anchor",
                None,
                &[("name", GuestValue::String(format!("bib-{}", key)))],
            )?;
            let entry = ctx.create_element(
                "BibEntry",
                None,
                &[
                    ("label", GuestValue::String(format!("[{}]", num))),
                    ("author", GuestValue::String(rec.author.clone())),
                    ("title", GuestValue::String(rec.title.clone())),
                    ("venue", GuestValue::String(rec.venue.clone())),
                    ("year", GuestValue::String(rec.year.clone())),
                ],
            )?;
            ctx.append_child(anchor, entry)?;
            ctx.append_child(bib_handle, anchor)?;
            key_to_num_and_anchor.insert(key.clone(), (num, anchor));
        }

        for (cite_handle, key) in cite_key_by_node {
            let parent = ctx.parent(cite_handle)?.ok_or_else(|| {
                DispatchError::Custom("Detached \\Cite node cannot be rewritten".into())
            })?;
            let (num, target_anchor) = key_to_num_and_anchor[&key];
            let link = ctx.create_element("Link", None, &[])?;
            ctx.set_text(link, &format!("[{}]", num))?;
            ctx.set_port_node_ref(link, "to", target_anchor)?;
            ctx.replace_child(parent, cite_handle, link)?;
        }

        Ok(())
    }
}

fn parse_bibtex_records(src: &str) -> BTreeMap<String, BibRecord> {
    let mut map = BTreeMap::new();
    for raw_entry in src.split('@').skip(1) {
        let Some(open) = raw_entry.find('{') else {
            continue;
        };
        let body = &raw_entry[open + 1..];
        let Some(comma) = body.find(',') else {
            continue;
        };
        let key = body[..comma].trim().to_string();
        let fields_str = &body[comma + 1..];
        let fields = parse_bib_fields(fields_str);
        map.insert(
            key.clone(),
            BibRecord {
                _key: key,
                author: fields.get("author").cloned().unwrap_or_default(),
                title: fields.get("title").cloned().unwrap_or_default(),
                venue: fields
                    .get("journal")
                    .or_else(|| fields.get("booktitle"))
                    .cloned()
                    .unwrap_or_default(),
                year: fields.get("year").cloned().unwrap_or_default(),
            },
        );
    }
    map
}

fn parse_bib_fields(s: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in s.lines() {
        let trimmed = line.trim().trim_end_matches([',', '}']);
        if let Some((k, v)) = trimmed.split_once('=') {
            let val = v.trim().trim_matches(['{', '}', '"']).to_string();
            out.insert(k.trim().to_lowercase(), val);
        }
    }
    out
}
