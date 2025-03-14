use crate::parser::MarkdownSubsection;
use crate::TranslationError;
use rusqlite::Connection;
use std::collections::HashMap;
use std::path::Path;

pub type CachedValues = HashMap<MarkdownSubsection, MarkdownSubsection>;

/// Caches translations in a SQLite database.
pub struct Cache {
    conn: Connection,
    src_lang_lc: String,
    dst_lang_lc: String,
}

impl Cache {
    pub fn new(db_path: &Path, src_lang: &str, dst_lang: &str) -> Result<Self, TranslationError> {
        let is_new = !db_path.exists();

        let conn = Connection::open(db_path)?;

        if is_new {
            conn.execute(
                "CREATE TABLE translated (
                    id           INTEGER PRIMARY KEY AUTOINCREMENT,
                    src_section  TEXT NOT NULL,
                    dst_section  TEXT NOT NULL,
                    src_lang_lc  TEXT NOT NULL,
                    dst_lang_lc  TEXT NOT NULL
                )",
                (),
            )?;
        };
        Ok(Self {
            conn,
            src_lang_lc: src_lang.trim().to_lowercase(),
            dst_lang_lc: dst_lang.trim().to_lowercase(),
        })
    }

    pub fn get(
        &self,
        src: &MarkdownSubsection,
    ) -> Result<Option<MarkdownSubsection>, TranslationError> {
        let query_res = self.conn.query_row(
            "SELECT dst_section
            FROM translated
            WHERE src_section = ?
              AND src_lang_lc = ?
              AND dst_lang_lc = ?",
            [&src.0, &self.src_lang_lc, &self.dst_lang_lc],
            |row| row.get::<_, String>(0),
        );

        match query_res {
            Ok(dst) => Ok(Some(MarkdownSubsection(dst))),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(TranslationError::DatabaseError(e)),
        }
    }

    /// Inserts a new cache entry unless it's a duplicate.
    pub fn insert(
        &mut self,
        src: MarkdownSubsection,
        dst: MarkdownSubsection,
    ) -> Result<(), TranslationError> {
        if self.get(&src)?.is_none() {
            self.conn.execute(
                "INSERT INTO translated (src_section, dst_section, src_lang_lc, dst_lang_lc)
                VALUES (?, ?, ?, ?)",
                [&src.0, &dst.0, &self.src_lang_lc, &self.dst_lang_lc],
            )?;
        }
        Ok(())
    }
}
