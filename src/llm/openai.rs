use super::{LLMBuilder, LLM};
use crate::parser::{MarkdownSection, MarkdownSubsection};
use crate::utils::substr_up_to_len;
use crate::{LLMError, TranslationConfig, MAX_LOG_SRC_LEN};
use anyhow::{anyhow, bail, Context};
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

const MAX_SEQUENTIAL_ERRORS: usize = 5;

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
            .with_api_key(&self.api_key);

        let client = Client::with_config(config);

        let asistants = {
            let client = client.clone();
            run_openai_request(async move || {
                client
                    .assistants()
                    .list(&HashMap::<String, String>::new())
                    .await
            }).await?
        };
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
            let client = client.clone();
            let assistant_id = assistant.id.clone();
            run_openai_request(async move || {
                client.assistants().update(&assistant_id, req.clone()).await
            }).await?
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
            let client = client.clone();
            run_openai_request(async move || {
                client.assistants().create(req.clone()).await
            }).await?
        };

        let thread = {
            let client = client.clone();
            run_openai_request(async move || {
                client.threads().create(CreateThreadRequest {
                    messages: None,
                    tool_resources: None,
                    metadata: None,
                }).await
            }).await?
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
            let client = client.clone();
            let cleanup_result = run_openai_request(async move || {
                client.threads().delete(&thread_id).await
            }).await;

            if let Err(e) = cleanup_result {
                log::error!("Failed to clean up thread: {:#?}", e);
            }
        });
    }
}

impl LLM for OpenAiGPT {
    async fn translate(&self, section: &MarkdownSection) -> Result<MarkdownSection, LLMError> {
        let mut subsections = vec![];
        for s in section.0.iter() {
            log::info!(r#"Sending message "{}...""#, substr_up_to_len(s.0.lines().next().unwrap(), MAX_LOG_SRC_LEN));
            let my_message = {
                let client = self.client.clone();
                let s = s.clone();
                let thread_id = self.thread.id.clone();
                run_openai_request(async move || {
                    client
                        .threads()
                        .messages(&thread_id)
                        .create(CreateMessageRequest {
                            role: MessageRole::User,
                            content: CreateMessageRequestContent::Content(s.0.clone()),
                            attachments: None,
                            metadata: None,
                        })
                        .await
                }).await?
            };
            log::info!("Message sent");

            let run_req = CreateRunRequest {
                assistant_id: self.assistant.id.clone(),
                ..Default::default()
            };

            log::info!("Getting translated message...");
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

                let client = self.client.clone();
                let thread_id = self.thread.id.clone();
                run_openai_request(async move || {
                    client
                        .threads()
                        .messages(&thread_id)
                        .list(&req)
                        .await
                }).await?
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
        let mut sequential_errors = 0;

        let mut backoff = ExponentialBackoff::default();

        'outer: loop {
            // Retry request, or bail out if we've hit the max number of sequential errors
            macro_rules! retry_or_bail {
                ($($t:tt)*) => {
                    if sequential_errors >= MAX_SEQUENTIAL_ERRORS {
                        bail!($($t)*);
                    } else {
                        log::warn!($($t)*);
                        sequential_errors += 1;
                        continue 'outer;
                    }
                };
            }

            // This is needed because OpenAI's wrapper library is awful at times
            macro_rules! wrap_request {
                ($do_req:expr, $msg:literal) => {{
                    let result = $do_req.await;
                    match result {
                        Ok(v) => v,
                        Err(OpenAIError::Reqwest(e)) => {
                            retry_or_bail!("{}, reqwest error: {e}", $msg);
                        }
                        Err(OpenAIError::JSONDeserialize(e)) => {
                            retry_or_bail!("{}, deserialization error: {e}", $msg);
                        }
                        e => return e.context($msg),
                    }
                }};
            }

            if let Some(duration) = backoff.next_backoff() {
                if duration > backoff.initial_interval {
                    log::warn!("Sleeping for {} ms", duration.as_millis());
                }
                tokio::time::sleep(duration).await;
            } else {
                bail!("Rate limit exceeded and backoff exhausted");
            }

            let mut run = wrap_request!(runs_api.create(req.clone()), "Failed to create run");
            loop {
                run =  wrap_request!(runs_api.retrieve(&run.id), "Failed to retrieve run");
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
                            retry_or_bail!("Server error: {message}")
                        }

                        None => {
                            retry_or_bail!("Run failed with no error")
                        }
                    },
                    RunStatus::Incomplete => {
                        retry_or_bail!(
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

/// This is needed because OpenAI's wrapper library is awful at times
async fn run_openai_request<R, F>(req: F) -> Result<R, LLMError>
where
    R: Send + Sync + 'static,
    F: AsyncFn() -> Result<R, OpenAIError> + 'static,
{
    let mut sequential_errors = 0;

    let mut backoff = ExponentialBackoff::default();

    'outer: loop {
        // Retry request, or bail out if we've hit the max number of sequential errors
        macro_rules! retry_or_bail {
            ($err:expr, $cxt:literal) => {
                let err = $err;
                if sequential_errors >= MAX_SEQUENTIAL_ERRORS {
                    return Err(err)
                        .context($cxt)
                        .map_err(LLMError::InteractionError);
                } else {
                    log::warn!("{}", err);
                    sequential_errors += 1;
                    if let Some(duration) = backoff.next_backoff() {
                        log::info!("Sleeping for {} ms", duration.as_millis());
                        tokio::time::sleep(duration).await;
                        continue 'outer;
                    }
                    return Err(err)
                        .context($cxt)
                        .context("Rate limit exceeded and backoff exhausted")
                        .map_err(LLMError::InteractionError);
                }
            };
        }

        let result = req().await;
        match result {
            Ok(v) => return Ok(v),
            Err(OpenAIError::Reqwest(e)) => {
                retry_or_bail!(e, "Reqwest error");
            }
            Err(OpenAIError::JSONDeserialize(e)) => {
                retry_or_bail!(e, "Deserialization error");
            }
            Err(e) => return Err(LLMError::InteractionError(e.into())),
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
