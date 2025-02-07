use super::{Generator, GeneratorBuilder};
use crate::parser::MarkdownSection;
use crate::TranslationError;

use itertools::Itertools;
use pandoc::OutputKind;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

pub struct PandocGeneratorBuilder;

impl GeneratorBuilder for PandocGeneratorBuilder {
    type Built = PandocGenrator;

    fn build(&self, output_path: &Path) -> Result<Self::Built, TranslationError> {
        let temp_dir = tempfile::tempdir().map_err(|e| TranslationError::IoError(e.into()))?;
        let temp_md_path = temp_dir.path().join("rosetta_translated.md");
        Ok(PandocGenrator {
            output_path: output_path.to_owned(),
            temp_md_path,
            temp_md_file: None,
            _temp_dir: temp_dir,
        })
    }
}

pub struct PandocGenrator {
    output_path: PathBuf,
    temp_md_path: PathBuf,
    temp_md_file: Option<File>,
    _temp_dir: TempDir,
}

impl Generator for PandocGenrator {
    fn write(&mut self, md: MarkdownSection) -> Result<(), TranslationError> {
        let temp_md_file = if let Some(file) = self.temp_md_file.as_mut() {
            file
        } else {
            let file = File::create(&self.temp_md_path)
                .map_err(|e| TranslationError::IoError(e.into()))?;
            self.temp_md_file = Some(file);
            self.temp_md_file.as_mut().unwrap()
        };

        temp_md_file
            .write_all(md.0.iter().map(|ss| &ss.0).join("\n").as_bytes())
            .map_err(|err| TranslationError::IoError(err))?;

        temp_md_file
            .write_all("\n\n".as_bytes())
            .map_err(|err| TranslationError::IoError(err))
    }

    fn finalize(&mut self) -> Result<(), TranslationError> {
        self.temp_md_file = None;

        let mut pandoc = pandoc::new();
        pandoc.add_input(&self.temp_md_path);
        pandoc.set_output(OutputKind::File(self.output_path.clone()));
        pandoc
            .execute()
            .map_err(|e| TranslationError::OtherError(e.into()))?;

        Ok(())
    }
}
