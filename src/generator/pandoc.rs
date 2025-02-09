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
        if translated_md_path.exists() {
            return Err(TranslationError::IoError(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("File already exists: {:?}", translated_md_path),
            )));
        }
        Ok(PandocGenrator {
            output_path: output_path.to_owned(),
            translated_md_path,
            translated_md_file: None,
        })
    }
}

pub struct PandocGenrator {
    output_path: PathBuf,
    translated_md_path: PathBuf,
    translated_md_file: Option<File>,
}

impl Generator for PandocGenrator {
    async fn write(&mut self, md: MarkdownSection) -> Result<(), TranslationError> {
        let temp_md_file = if let Some(file) = self.translated_md_file.as_mut() {
            file
        } else {
            let file = File::create(&self.translated_md_path)
                .await
                .map_err(|e| TranslationError::IoError(e.into()))?;
            self.translated_md_file = Some(file);
            self.translated_md_file.as_mut().unwrap()
        };

        temp_md_file
            .write_all(md.0.iter().map(|ss| &ss.0).join("\n").as_bytes())
            .await
            .map_err(|err| TranslationError::IoError(err))?;

        temp_md_file
            .write_all("\n\n".as_bytes())
            .await
            .map_err(|err| TranslationError::IoError(err))
    }

    async fn finalize(&mut self) -> Result<(), TranslationError> {
        self.translated_md_file = None;
        let temp_md_path = self.translated_md_path.clone();
        let output_path = self.output_path.clone();
        tokio::task::spawn_blocking(move || {
            let mut pandoc = pandoc::new();
            pandoc.add_input(&temp_md_path);
            pandoc.set_output(OutputKind::File(output_path));
            pandoc.execute()
        })
        .await
        .map_err(|e| TranslationError::OtherError(e.into()))?
        .map_err(|e| TranslationError::OtherError(e.into()))?;

        Ok(())
    }
}
