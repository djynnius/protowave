//! Full-text search (FR-29..30) over blip text, via embedded tantivy —
//! the successor of legacy Wave's Lucene index.
//!
//! Indexing is incremental: the engine's change stream triggers a per-wave
//! re-index of extracted text (O(changed wave), coalesced). ACL filtering
//! happens at query time against the caller's wave set.

use std::collections::HashSet;
use std::io;
use std::path::Path;
use std::sync::Mutex;

use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::{Field, Schema, STORED, STRING, TEXT};
use tantivy::{doc, Index, IndexWriter, Term};

#[derive(Debug, Clone, serde::Serialize)]
pub struct SearchHit {
    pub wave: String,
    pub title: String,
    pub snippet: String,
}

pub trait SearchIndex: Send + Sync + 'static {
    fn upsert(&self, wave: &str, title: &str, body: &str) -> io::Result<()>;
    fn query(&self, q: &str, allowed: &HashSet<String>, limit: usize)
        -> io::Result<Vec<SearchHit>>;
}

fn other(e: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::Other, e.to_string())
}

pub struct TantivyIndex {
    index: Index,
    writer: Mutex<IndexWriter>,
    wave: Field,
    title: Field,
    body: Field,
}

impl TantivyIndex {
    pub fn open(dir: &Path) -> io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let mut builder = Schema::builder();
        let wave = builder.add_text_field("wave", STRING | STORED);
        let title = builder.add_text_field("title", TEXT | STORED);
        let body = builder.add_text_field("body", TEXT | STORED);
        let schema = builder.build();
        let index = Index::open_or_create(
            tantivy::directory::MmapDirectory::open(dir).map_err(other)?,
            schema,
        )
        .map_err(other)?;
        let writer = index.writer(15_000_000).map_err(other)?;
        Ok(Self {
            index,
            writer: Mutex::new(writer),
            wave,
            title,
            body,
        })
    }
}

impl SearchIndex for TantivyIndex {
    fn upsert(&self, wave: &str, title: &str, body: &str) -> io::Result<()> {
        let mut writer = self.writer.lock().unwrap();
        writer.delete_term(Term::from_field_text(self.wave, wave));
        writer
            .add_document(doc!(
                self.wave => wave,
                self.title => title,
                self.body => body,
            ))
            .map_err(other)?;
        writer.commit().map_err(other)?;
        Ok(())
    }

    fn query(
        &self,
        q: &str,
        allowed: &HashSet<String>,
        limit: usize,
    ) -> io::Result<Vec<SearchHit>> {
        let reader = self.index.reader().map_err(other)?;
        let searcher = reader.searcher();
        let parser = QueryParser::for_index(&self.index, vec![self.title, self.body]);
        let query = match parser.parse_query(q) {
            Ok(query) => query,
            Err(_) => return Ok(Vec::new()), // malformed query is just no hits
        };
        // Over-fetch so ACL filtering still fills the page (NFR-C6 scale is
        // fine: limit is small and constant).
        let top = searcher
            .search(&query, &TopDocs::with_limit(limit * 4))
            .map_err(other)?;

        let snippets = tantivy::SnippetGenerator::create(&searcher, &query, self.body).ok();
        let mut hits = Vec::new();
        for (_score, addr) in top {
            if hits.len() >= limit {
                break;
            }
            let doc = searcher.doc(addr).map_err(other)?;
            let field_text = |f: Field| {
                doc.get_first(f)
                    .and_then(|v| v.as_text())
                    .unwrap_or_default()
                    .to_string()
            };
            let wave = field_text(self.wave);
            if !allowed.contains(&wave) {
                continue;
            }
            let snippet = snippets
                .as_ref()
                .map(|g| g.snippet_from_doc(&doc).to_html())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| {
                    let body = field_text(self.body);
                    body.chars().take(140).collect()
                });
            hits.push(SearchHit {
                wave,
                title: field_text(self.title),
                snippet,
            });
        }
        Ok(hits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_query_acl_and_update() {
        let dir = tempfile::tempdir().unwrap();
        let idx = TantivyIndex::open(dir.path()).unwrap();
        idx.upsert("d/w1", "Plans", "we sail at dawn toward the harbor")
            .unwrap();
        idx.upsert("d/w2", "Secrets", "the harbor is a trap")
            .unwrap();

        let mine: HashSet<String> = ["d/w1".to_string()].into();
        let hits = idx.query("harbor", &mine, 10).unwrap();
        assert_eq!(hits.len(), 1, "ACL filters out w2");
        assert_eq!(hits[0].wave, "d/w1");

        // Re-upsert replaces the old document.
        idx.upsert("d/w1", "Plans", "no more sailing").unwrap();
        let hits = idx.query("harbor", &mine, 10).unwrap();
        assert!(hits.is_empty());
        let hits = idx.query("sailing", &mine, 10).unwrap();
        assert_eq!(hits.len(), 1);
    }
}
