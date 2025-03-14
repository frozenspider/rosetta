use super::{Generator, GeneratorBuilder};
use crate::parser::MarkdownSection;
use crate::TranslationError;

use itertools::Itertools;
use pandoc::OutputKind;
use std::path::{Path, PathBuf};
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

pub struct PandocGeneratorBuilder;

impl GeneratorBuilder for PandocGeneratorBuilder {
    type Built = PandocGenrator;

    async fn build(&self, output_path: &Path) -> Result<Self::Built, TranslationError> {
        let translated_md_path = output_path.with_extension("md");

        // Created from scratch every time
        let translated_md_file = File::create(&translated_md_path)
            .await
            .map_err(TranslationError::IoError)?;

        Ok(PandocGenrator {
            output_path: output_path.to_owned(),
            translated_md_path,
            translated_md_file,
        })
    }
}

pub struct PandocGenrator {
    output_path: PathBuf,
    translated_md_path: PathBuf,
    translated_md_file: File,
}

impl Generator for PandocGenrator {
    async fn write(&mut self, md: MarkdownSection) -> Result<(), TranslationError> {
        self.translated_md_file
            .write_all(md.0.iter().map(|ss| &ss.0).join("\n").as_bytes())
            .await?;

        self.translated_md_file
            .write_all("\n\n".as_bytes())
            .await?;

        Ok(())
    }

    async fn finalize(&mut self) -> Result<(), TranslationError> {
        let translated_md_path = self.translated_md_path.clone();
        let output_path = self.output_path.clone();

        // If output file itself is Markdown, no need to run pandoc
        if translated_md_path != output_path {
            tokio::task::spawn_blocking(move || {
                let mut pandoc = pandoc::new();
                pandoc.add_input(&translated_md_path);
                pandoc.set_output(OutputKind::File(output_path));
                pandoc.execute()
            })
            .await
            .map_err(|e| TranslationError::OtherError(e.into()))?
            .map_err(|e| TranslationError::OtherError(e.into()))?;
        }

        Ok(())
    }
}
