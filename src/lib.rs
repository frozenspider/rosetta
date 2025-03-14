#![allow(async_fn_in_trait)]

pub mod cache;
pub mod generator;
pub mod llm;
pub mod parser;
pub mod utils;

use crate::generator::{Generator, GeneratorBuilder};
use crate::llm::{LLMBuilder, LLM};
use crate::parser::{MarkdownSection, MarkdownSubsection, Parser};
use config::Config;
use std::fmt::Display;
use std::fs;
use std::path::Path;
use crate::cache::Cache;
use crate::utils::substr_up_to_len;

pub const MAX_LOG_SRC_LEN: usize = 100;

pub async fn translate(
    settings: Config,
    input: &Path,
    output: &Path,
    cfg: TranslationConfig,
    send_progress: impl SendProgress,
) -> Result<(), TranslationError> {
    let parser = parser::pandoc::PandocParser {
        max_section_len: cfg.max_section_len,
        skip_if_present: true
    };

    let api_key = settings
        .get_string("openai.api_key")
        .map_err(|e| TranslationError::OtherError(anyhow::Error::new(e)))?;

    let model =
        settings
        .get_string("openai.model")
        .map_err(|e| TranslationError::OtherError(anyhow::Error::new(e)))?;

    let llm_builder = llm::openai::OpenAiGPTBuilder::new(model, api_key);

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
    pub additional_instructions: String,
    pub max_section_len: usize,
}

impl Default for TranslationConfig {
    fn default() -> Self {
        TranslationConfig {
            src_lang: "English".to_owned(),
            dst_lang: "Russian".to_owned(),
            subject: "Unknown".to_owned(),
            tone: "formal".to_owned(),
            additional_instructions: "".to_owned(),
            max_section_len: 5000
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
pub enum LLMError {
    ConnectionError(anyhow::Error),
    ApiError(anyhow::Error),
    InteractionError(anyhow::Error),
    OtherError(anyhow::Error),
}

#[derive(Debug)]
pub enum TranslationError {
    ParseError(ParseError),
    IoError(std::io::Error),
    DatabaseError(rusqlite::Error),
    LLMError(LLMError),
    OtherError(anyhow::Error),
}

impl From<rusqlite::Error> for TranslationError {
    fn from(e: rusqlite::Error) -> Self {
        TranslationError::DatabaseError(e)
    }
}

impl From<std::io::Error> for TranslationError {
    fn from(e: std::io::Error) -> Self {
        TranslationError::IoError(e)
    }
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
            TranslationError::DatabaseError(e) => {
                write!(f, "Database error: {}", e)
            }
            TranslationError::LLMError(LLMError::ConnectionError(e)) => {
                write!(f, "LLM connection error: {}", e)
            }
            TranslationError::LLMError(LLMError::ApiError(e)) => {
                write!(f, "LLM API error: {}", e)
            }
            TranslationError::LLMError(LLMError::InteractionError(e)) => {
                write!(f, "LLM interaction error: {}", e)
            }
            TranslationError::LLMError(LLMError::OtherError(e)) => {
                write!(f, "Unexpected LLM error: {}", e)
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
        fs::create_dir_all(output.parent().expect("output parent"))
            .map_err(TranslationError::IoError)?;

        let input_sections = self
            .parser
            .parse(input)
            .await
            .map_err(TranslationError::ParseError)?;
        let total_sections = input_sections.len();

        let mut cache = Cache::new(&output.with_extension("sqlite"), &cfg.src_lang, &cfg.dst_lang)?;

        let mut generator =
            self.generator_builder.build(output).await?;

        {
            let llm = self
                .llm_builder
                .build(cfg)
                .await
                .map_err(TranslationError::LLMError)?;

            for (current, section) in input_sections.into_iter().enumerate() {
                let cached_subsections = section.0.iter()
                    .map(|ss| cache.get(ss))
                    .collect::<Result<Vec<Option<MarkdownSubsection>>, TranslationError>>()?;

                let translated_section =
                    if cached_subsections.iter().all(|opt| opt.is_some()) {
                        // Translation is fully cached
                        let translated = MarkdownSection(cached_subsections.into_iter().map(|opt| opt.unwrap()).collect());
                        log::info!("Section {} already translated:\n >>> {}\n <<< {}", current,
                            substr_up_to_len(section.0.first().unwrap().0.lines().next().unwrap(), MAX_LOG_SRC_LEN),
                            substr_up_to_len(translated.0.first().unwrap().0.lines().next().unwrap(), MAX_LOG_SRC_LEN));
                        translated
                    } else {
                        let translated = llm
                            .translate(&section)
                            .await
                            .map_err(TranslationError::LLMError)?;

                        for (src, dst) in section.0.iter().zip(translated.0.iter()) {
                            cache.insert(src.clone(), dst.clone())?;
                        }

                        translated
                    };

                generator.write(translated_section).await?;

                self.send_progress.send_progress(Progress {
                    processed_sections: current + 1,
                    total_sections,
                });
            }
        }

        generator.finalize().await?;

        Ok(())
    }
}
