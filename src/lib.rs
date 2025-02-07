pub mod llm;
pub mod parser;

use crate::llm::{LLMBuilder, LLM};
use crate::parser::Parser;
use std::fs;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::mpsc::Sender;

pub fn my_fn() {
    println!("Hello, world!");
}

pub struct TranslationConfig {
    str_lang: String,
    dst_lang: String,
    subject: String,
    tone: String,
}

pub trait TranslationService {
    fn translate(
        &self,
        input: &Path,
        output: &Path,
        cfg: TranslationConfig,
    ) -> Result<(), TranslationError>;
}

#[derive(Debug)]
pub enum ParseError {
    UnsupportedFormatError { supported_formats: Vec<String> },
    OtherError(anyhow::Error),
}

#[derive(Debug)]
pub enum TranslationError {
    ParseError(ParseError),
    IoError(std::io::Error),
    OtherError(anyhow::Error),
}

#[derive(Debug)]
pub struct Progress {
    pub processed_sections: usize,
    pub total_sections: usize,
}

pub struct LlmTranslationService<P: Parser, LB: LLMBuilder> {
    parser: P,
    llm_builder: LB,
    progress_sender: Option<Sender<Progress>>,
}

impl<P: Parser, LB: LLMBuilder> TranslationService for LlmTranslationService<P, LB> {
    fn translate(
        &self,
        input: &Path,
        output: &Path,
        cfg: TranslationConfig,
    ) -> Result<(), TranslationError> {
        if !input.exists() {
            return Err(TranslationError::IoError(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("File not found: {:?}", input),
            )));
        }
        if output.exists() {
            return Err(TranslationError::IoError(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("File already exists: {:?}", output),
            )));
        }
        let mut out = File::create(output).map_err(|err| TranslationError::IoError(err))?;

        fs::create_dir_all(output.parent().expect("output parent"))
            .map_err(|err| TranslationError::IoError(err))?;

        let input_sections = self
            .parser
            .parse(input)
            .map_err(|err| TranslationError::ParseError(err))?;
        let total_sections = input_sections.len();

        let llm = self
            .llm_builder
            .build(cfg)
            .map_err(|err| TranslationError::OtherError(err))?;

        for (current, section) in input_sections.into_iter().enumerate() {
            let translated_section = llm.translate(section)?;

            out.write_all(translated_section.as_bytes())
                .map_err(|err| TranslationError::IoError(err))?;

            if let Some(sender) = self.progress_sender.as_ref() {
                sender
                    .send(Progress {
                        processed_sections: current + 1,
                        total_sections,
                    })
                    .expect("progress send");
            }
        }

        Ok(())
    }
}
