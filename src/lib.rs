#![allow(async_fn_in_trait)]

pub mod generator;
pub mod llm;
pub mod parser;

use crate::generator::{Generator, GeneratorBuilder};
use crate::llm::{LLMBuilder, LLM};
use crate::parser::Parser;
use std::fmt::Display;
use std::fs;
use std::path::Path;

pub async fn translate(
    input: &Path,
    output: &Path,
    cfg: TranslationConfig,
    send_progress: impl SendProgress,
) -> Result<(), TranslationError> {
    let parser = parser::pandoc::PandocParser {
        max_section_len: 6000,
    };

    let llm_builder = llm::dummy::DummyLLMBuilder;

    let generator_builder = generator::pandoc::PandocGeneratorBuilder;

    let translator = LlmTranslationService {
        parser,
        llm_builder,
        generator_builder,
        send_progress,
    };

    translator.translate(input, output, cfg).await
}

#[derive(Debug, Clone)]
pub struct TranslationConfig {
    pub src_lang: String,
    pub dst_lang: String,
    pub subject: String,
    pub tone: String,
}

impl Default for TranslationConfig {
    fn default() -> Self {
        TranslationConfig {
            src_lang: "English".to_owned(),
            dst_lang: "Spanish".to_owned(),
            subject: "Unknown".to_owned(),
            tone: "formal".to_owned(),
        }
    }
}

pub trait TranslationService {
    async fn translate(
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

impl Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::UnsupportedFormatError { supported_formats } => {
                write!(
                    f,
                    "Unsupported format. Supported formats: {:?}",
                    supported_formats
                )
            }
            ParseError::OtherError(e) => {
                write!(f, "{}", e)
            }
        }
    }
}

#[derive(Debug)]
pub enum TranslationError {
    ParseError(ParseError),
    IoError(std::io::Error),
    OtherError(anyhow::Error),
}

impl Display for TranslationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TranslationError::ParseError(e) => {
                write!(f, "Parsing failed: {}", e)
            }
            TranslationError::IoError(e) => {
                write!(f, "IO error: {}", e)
            }
            TranslationError::OtherError(e) => {
                write!(f, "Error: {}", e)
            }
        }
    }
}

#[derive(Debug)]
pub enum TranslationStatus {
    Started,
    Progress(Progress),
    Success,
    Error(TranslationError),
}

#[derive(Debug, Clone)]
pub struct Progress {
    pub processed_sections: usize,
    pub total_sections: usize,
}

pub trait SendProgress: Send + Sync {
    fn send_progress(&self, progress: Progress);
}

pub struct DummySendProgress;

impl SendProgress for DummySendProgress {
    fn send_progress(&self, _progress: Progress) {}
}

pub struct LlmTranslationService<P, LB, GB, SP> {
    parser: P,
    llm_builder: LB,
    generator_builder: GB,
    send_progress: SP,
}

impl<P, LB, GB, SP> TranslationService for LlmTranslationService<P, LB, GB, SP>
where
    P: Parser,
    LB: LLMBuilder,
    GB: GeneratorBuilder,
    SP: SendProgress,
{
    async fn translate(
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

        fs::create_dir_all(output.parent().expect("output parent"))
            .map_err(|err| TranslationError::IoError(err))?;

        let input_sections = self
            .parser
            .parse(input)
            .await
            .map_err(|err| TranslationError::ParseError(err))?;
        let total_sections = input_sections.len();

        let llm = self
            .llm_builder
            .build(cfg)
            .await
            .map_err(|err| TranslationError::OtherError(err))?;

        let mut gen = self.generator_builder.build(output).await?;

        for (current, section) in input_sections.into_iter().enumerate() {
            let translated_section = llm.translate(section).await?;

            gen.write(translated_section).await?;

            self.send_progress.send_progress(Progress {
                processed_sections: current + 1,
                total_sections,
            });
        }

        gen.finalize().await?;

        Ok(())
    }
}
