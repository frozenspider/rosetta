use super::{LLMBuilder, LLM};
use crate::parser::{MarkdownSection, MarkdownSubsection};
use crate::{LLMError, TranslationConfig};
use anyhow::{anyhow, bail};
use async_openai::config::OpenAIConfig;
use async_openai::error::OpenAIError;
use async_openai::types::{
    AssistantObject, AssistantsApiResponseFormatOption, CreateAssistantRequest,
    CreateMessageRequest, CreateMessageRequestContent, CreateRunRequest, CreateThreadRequest,
    LastError, LastErrorCode, MessageContent, MessageRole, ModifyAssistantRequest, ResponseFormat,
    RunObject, RunStatus, ThreadObject,
};
use async_openai::Client;
use backoff::backoff::Backoff;
use backoff::ExponentialBackoff;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::error::Error;

const ASSISTANT_NAME: &str = "rosetta-translator";
const ASSISTANT_DESC: &str = "A Rosetta translation assistant";

const SLEEP_TIME_MS: u64 = 2 * 1000;

pub struct OpenAiGPTBuilder {
    model: String,
    api_key: String,
    temperature: f32,
    top_p: f32,
}

/// Builder for OpenAI-compatible LLM APIs
impl OpenAiGPTBuilder {
    pub fn new(model: String, api_key: String) -> Self {
        OpenAiGPTBuilder {
            model,
            api_key,
            temperature: 1.0,
            top_p: 1.0,
        }
    }
}

impl LLMBuilder for OpenAiGPTBuilder {
    type Built = OpenAiGPT;

    async fn build(&self, cfg: TranslationConfig) -> Result<Self::Built, LLMError> {
        let prompt = super::cfg_to_prompt(&cfg);

        let config = OpenAIConfig::new()
            .with_api_key(&self.api_key)
            .with_project_id("rosetta");

        let client = Client::with_config(config);

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

            let mut run_req = CreateRunRequest::default();
            run_req.assistant_id = self.assistant.id.clone();

            let run = self
                .run_with_backoff(run_req)
                .await
                .map_err(LLMError::InteractionError)?;

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

impl OpenAiGPT {
    async fn run_with_backoff(&self, req: CreateRunRequest) -> Result<RunObject, anyhow::Error> {
        let runs_api = self.client.threads();
        let runs_api = runs_api.runs(&self.thread.id);

        let mut backoff = ExponentialBackoff::default();
        backoff.initial_interval = std::time::Duration::from_millis(SLEEP_TIME_MS);

        'outer: loop {
            if let Some(duration) = backoff.next_backoff() {
                if duration > backoff.initial_interval {
                    log::warn!("Sleeping for {} seconds", duration.as_secs());
                }
                tokio::time::sleep(duration).await;
            } else {
                bail!("Rate limit exceeded and backoff exhausted");
            }

            let mut run = runs_api.create(req.clone()).await?;
            loop {
                run = runs_api.retrieve(&run.id).await?;
                match run.status {
                    RunStatus::Completed => {
                        log::info!("Run complete");
                        return Ok(run);
                    }
                    RunStatus::Queued | RunStatus::InProgress => { /* NOOP */ }
                    RunStatus::Cancelling | RunStatus::Cancelled => {
                        bail!("Run is cancelled!")
                    }
                    RunStatus::Failed => match run.last_error {
                        Some(LastError {
                            code: LastErrorCode::RateLimitExceeded,
                            message,
                        }) => {
                            log::warn!("Hit the rate limit: {message}");
                            continue 'outer;
                        }
                        Some(LastError {
                            code: LastErrorCode::InvalidPrompt,
                            message,
                        }) => {
                            bail!("Invalid prompt: {message}")
                        }

                        Some(LastError {
                            code: LastErrorCode::ServerError,
                            message,
                        }) => {
                            bail!("Server error: {message}")
                        }

                        None => {
                            bail!("Run failed with no error")
                        }
                    },
                    RunStatus::Incomplete => {
                        bail!(
                            "Run is incomplete: {:?}",
                            run.incomplete_details.unwrap().reason
                        )
                    }
                    RunStatus::Expired => {
                        bail!("Run expired!")
                    }
                    RunStatus::RequiresAction => {
                        unreachable!("No tools should be needed")
                    }
                }
            }
        }
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
