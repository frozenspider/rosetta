use super::{LLMBuilder, LLM};
use crate::parser::{MarkdownSection, MarkdownSubsection};
use crate::{LLMError, TranslationConfig};
use anyhow::anyhow;
use async_openai::config::OpenAIConfig;
use async_openai::error::OpenAIError;
use async_openai::types::{
    AssistantObject, AssistantsApiResponseFormatOption, CreateAssistantRequest,
    CreateMessageRequest, CreateMessageRequestContent, CreateRunRequest, CreateThreadRequest,
    LastError, LastErrorCode, MessageContent, MessageRole, ModifyAssistantRequest, ResponseFormat,
    RunStatus, ThreadObject,
};
use async_openai::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::error::Error;

const ASSISTANT_NAME: &str = "rosetta-translator";
const ASSISTANT_DESC: &str = "A Rosetta translation assistant";

const SLEEP_TIME_MS: u64 = 2000;

pub struct OpenAiGPTBuilder {
    api_key: String,
    model: String,
    temperature: f32,
    top_p: f32,
}

impl OpenAiGPTBuilder {
    pub fn new(api_key: String) -> Self {
        OpenAiGPTBuilder {
            api_key,
            model: "gpt-4o".to_owned(),
            temperature: 1.0,
            top_p: 1.0,
        }
    }
}

impl LLMBuilder for OpenAiGPTBuilder {
    type Built = OpenAiGPT;

    async fn build(&self, cfg: TranslationConfig) -> Result<Self::Built, LLMError> {
        let prompt = format!(
            r#"
You are a professional translator from {} language to {}.
Translate each of my messages, keeping in mind that they are pieces of the same text.
The subject of the source text is "{}"
Make sure this translation is accurate and natural, preserve Markdown syntax.
Translation tone needs to be matching the source, use {} tone when in doubt.
Output just the translation and nothing else.
"#,
            cfg.src_lang, cfg.dst_lang, cfg.subject, cfg.tone
        )
        .trim()
        .to_owned();

        const API_KEY_VARNAME: &str = "OPENAI_API_KEY";

        env::set_var(API_KEY_VARNAME, &self.api_key);
        let client = Client::new();
        env::remove_var(API_KEY_VARNAME);

        let asistants = client
            .assistants()
            .list(&HashMap::<String, String>::new())
            .await?;
        assert!(!asistants.has_more);

        let assistant = asistants
            .data
            .into_iter()
            .find(|assistant| assistant.name.as_deref() == Some(ASSISTANT_NAME));

        let assistant = if let Some(assistant) = assistant {
            let req = ModifyAssistantRequest {
                model: Some(self.model.clone()),
                name: Some(ASSISTANT_NAME.to_owned()),
                description: Some(ASSISTANT_DESC.to_owned()),
                instructions: Some(prompt),
                tools: None,
                tool_resources: None,
                metadata: None,
                temperature: Some(self.temperature),
                top_p: Some(self.top_p),
                response_format: Some(AssistantsApiResponseFormatOption::Format(
                    ResponseFormat::Text,
                )),
            };
            client.assistants().update(&assistant.id, req).await?
        } else {
            let req = CreateAssistantRequest {
                model: self.model.clone(),
                name: Some(ASSISTANT_NAME.to_owned()),
                description: Some(ASSISTANT_DESC.to_owned()),
                instructions: Some(prompt),
                tools: None,
                tool_resources: None,
                metadata: None,
                temperature: Some(self.temperature),
                top_p: Some(self.top_p),
                response_format: Some(AssistantsApiResponseFormatOption::Format(
                    ResponseFormat::Text,
                )),
            };
            client.assistants().create(req).await?
        };

        let thread = {
            let req = CreateThreadRequest {
                messages: None,
                tool_resources: None,
                metadata: None,
            };
            client.threads().create(req).await?
        };

        Ok(OpenAiGPT {
            client,
            assistant,
            thread,
        })
    }
}

pub struct OpenAiGPT {
    client: Client<OpenAIConfig>,
    assistant: AssistantObject,
    thread: ThreadObject,
}

impl Drop for OpenAiGPT {
    fn drop(&mut self) {
        let client = self.client.clone();
        let thread_id = self.thread.id.clone();
        tokio::spawn(async move {
            client
                .threads()
                .delete(&thread_id)
                .await
                .expect("thread cleanup");
        });
    }
}

impl LLM for OpenAiGPT {
    async fn translate(&self, section: MarkdownSection) -> Result<MarkdownSection, LLMError> {
        let mut subsections = vec![];
        for s in section.0 {
            log::info!(r#"Sending message "{}...""#, {
                let line = s.0.lines().next().unwrap();
                if line.len() > 20 {
                    &line[..20]
                } else {
                    line
                }
            });
            let my_message = {
                self.client
                    .threads()
                    .messages(&self.thread.id)
                    .create(CreateMessageRequest {
                        role: MessageRole::User,
                        content: CreateMessageRequestContent::Content(s.0),
                        attachments: None,
                        metadata: None,
                    })
                    .await?
            };

            let runs_api = self.client.threads();
            let runs_api = runs_api.runs(&self.thread.id);

            let mut run_req = CreateRunRequest::default();
            run_req.assistant_id = self.assistant.id.clone();
            let mut run = runs_api.create(run_req).await?;
            loop {
                run = runs_api.retrieve(&run.id).await?;
                match (run.status, run.last_error) {
                    (RunStatus::Completed, _) => {
                        log::info!("Run complete");
                        break;
                    }
                    (
                        _,
                        Some(LastError {
                            code: LastErrorCode::RateLimitExceeded,
                            ..
                        }),
                    ) => {
                        log::warn!("Hit the rate limit");
                        tokio::time::sleep(std::time::Duration::from_millis(SLEEP_TIME_MS * 3))
                            .await;
                    }
                    (RunStatus::Queued | RunStatus::InProgress, _) => { /* NOOP */ }
                    (RunStatus::Cancelling | RunStatus::Cancelled, _) => {
                        return Err(LLMError::InteractionError(anyhow!("Run is cancelled!")))
                    }
                    (
                        RunStatus::Failed,
                        Some(LastError {
                            code: LastErrorCode::InvalidPrompt,
                            message,
                        }),
                    ) => {
                        return Err(LLMError::InteractionError(anyhow!(
                            "Invalid prompt: {message}"
                        )))
                    }
                    (
                        RunStatus::Failed,
                        Some(LastError {
                            code: LastErrorCode::ServerError,
                            message,
                        }),
                    ) => {
                        return Err(LLMError::InteractionError(anyhow!(
                            "Server error: {message}"
                        )));
                    }
                    (RunStatus::Failed, None) => {
                        return Err(LLMError::InteractionError(anyhow!(
                            "Run failed with no error"
                        )));
                    }
                    (RunStatus::Incomplete, _) => {
                        return Err(LLMError::InteractionError(anyhow!(
                            "Run is incomplete: {:?}",
                            run.incomplete_details.unwrap().reason
                        )))
                    }
                    (RunStatus::Expired, _) => {
                        return Err(LLMError::InteractionError(anyhow!("Run expired!")))
                    }
                    (RunStatus::RequiresAction, _) => {
                        unreachable!("No tools should be needed")
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(SLEEP_TIME_MS)).await;
            }
            let msgs = {
                let req = ListMessagesRequest {
                    run_id: Some(run.id.clone()),
                    limit: None,
                    order: Some("asc".to_owned()),
                    after: Some(my_message.id.clone()),
                    before: None,
                };
                self.client
                    .threads()
                    .messages(&self.thread.id)
                    .list(&req)
                    .await?
            };
            assert!(!msgs.has_more);

            if msgs.data.len() != 1 {
                return Err(LLMError::InteractionError(anyhow!(
                    "Incorrect number of response messages: {}",
                    msgs.data.len()
                )));
            }
            let msg = &msgs.data[0];

            if msg.content.len() != 1 {
                return Err(LLMError::InteractionError(anyhow!(
                    "Incorrect number of response message sections: {}",
                    msgs.data.len()
                )));
            }
            let mc = &msg.content[0];
            let translated = match mc {
                MessageContent::Text(obj) => obj.text.value.clone(),
                _ => {
                    return Err(LLMError::InteractionError(anyhow!(
                        "Incorrect response type: {:?}",
                        mc
                    )))
                }
            };
            subsections.push(MarkdownSubsection(translated));
        }
        Ok(MarkdownSection(subsections))
    }
}

impl From<OpenAIError> for LLMError {
    fn from(err: OpenAIError) -> Self {
        match err {
            OpenAIError::Reqwest(e) => LLMError::ConnectionError(if let Some(e) = e.source() {
                anyhow!("{e}")
            } else {
                e.into()
            }),
            OpenAIError::ApiError(e) => LLMError::ApiError(anyhow!("{e}")),
            OpenAIError::JSONDeserialize(e) => LLMError::OtherError(e.into()),
            OpenAIError::FileSaveError(e) => LLMError::OtherError(anyhow!("{e}")),
            OpenAIError::FileReadError(e) => LLMError::OtherError(anyhow!("{e}")),
            OpenAIError::StreamError(e) => LLMError::ConnectionError(anyhow!("{e}")),
            OpenAIError::InvalidArgument(e) => LLMError::OtherError(anyhow!("{e}")),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ListMessagesRequest {
    /// Filter messages by the run ID that generated them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,

    /// A limit on the number of objects to be returned. Limit can range between 1 and 100, and the default is 20.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,

    /// Sort order by the created_at timestamp of the objects. asc for ascending order and desc for descending order.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<String>,

    /// A cursor for use in pagination. after is an object ID that defines your place in the list.
    /// For instance, if you make a list request and receive 100 objects, ending with obj_foo,
    /// your subsequent call can include after=obj_foo in order to fetch the next page of the list.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,

    /// A cursor for use in pagination. before is an object ID that defines your place in the list.
    /// For instance, if you make a list request and receive 100 objects, starting with obj_foo,
    /// your subsequent call can include before=obj_foo in order to fetch the previous page of the list.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
}
