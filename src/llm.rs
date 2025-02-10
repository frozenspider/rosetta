pub mod dummy;
pub mod openai;

use super::parser::MarkdownSection;
use super::{LLMError, TranslationConfig};

pub trait LLMBuilder {
    type Built: LLM;

    async fn build(&self, cfg: TranslationConfig) -> Result<Self::Built, LLMError>;
}

pub trait LLM {
    async fn translate(&self, section: MarkdownSection) -> Result<MarkdownSection, LLMError>;
}

fn cfg_to_prompt(cfg: &TranslationConfig) -> String {
    format!(
        r#"
You are a professional translator from {} language to {}.
Translate each of my messages, keeping in mind that they are pieces of the same text.
The subject of the source text is "{}"
Make sure this translation is accurate and natural, preserve Markdown syntax.
Translation tone needs to be matching the source, use {} tone when in doubt.
{}.
Output just the translation and nothing else.
"#,
        cfg.src_lang, cfg.dst_lang, cfg.subject, cfg.tone, cfg.additional_instructions
    )
    .trim()
    .to_owned()
}
