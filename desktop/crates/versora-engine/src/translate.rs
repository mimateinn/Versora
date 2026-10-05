//! Numbered translation batches, stable deduplication and structural retry policy.
use crate::providers::{check_cancel, CancelToken, ProviderConfig, ProviderError, ProviderRuntime};
use serde::{Deserialize, Serialize};
use std::{future::Future, pin::Pin, sync::Arc};
use versora_core::codec::{self, CodecError, Shape, DEFAULT_MARK};

pub type Progress = Arc<dyn Fn(usize, usize) + Send + Sync>;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct TranslationOptions {
    pub target_lang: String,
    pub max_chars: usize,
    pub global_limit: usize,
    pub per_provider_limit: usize,
}
impl Default for TranslationOptions {
    fn default() -> Self {
        Self {
            target_lang: "en".into(),
            max_chars: 3000,
            global_limit: 3,
            per_provider_limit: 1,
        }
    }
}

pub trait BatchCaller: Send + Sync {
    fn call<'a>(
        &'a self,
        system: &'a str,
        user: &'a str,
        cancel: &'a CancelToken,
    ) -> Pin<Box<dyn Future<Output = Result<String, ProviderError>> + Send + 'a>>;
}
struct RuntimeCaller<'a> {
    runtime: &'a ProviderRuntime,
    configs: &'a [ProviderConfig],
}
impl BatchCaller for RuntimeCaller<'_> {
    fn call<'a>(
        &'a self,
        system: &'a str,
        user: &'a str,
        cancel: &'a CancelToken,
    ) -> Pin<Box<dyn Future<Output = Result<String, ProviderError>> + Send + 'a>> {
        Box::pin(self.runtime.call(self.configs, system, user, cancel))
    }
}

enum BatchError {
    Parse(CodecError),
    Provider(ProviderError),
}
const REASK:&str="\nThe previous answer broke a one-line item: return exactly one numbered item on one physical line, with no added line breaks. Keep all literal line-break marks.";

fn parsed(shapes: &[Shape], reply: &str, mark: char) -> Result<Vec<String>, CodecError> {
    let raw = match codec::parse_raw_numbered(reply, shapes.len()) {
        Ok(raw) => raw,
        Err(CodecError::TextBeforeFirstItem) if shapes.len() == 1 => {
            return shapes[0]
                .apply_text(&codec::single_unnumbered(reply, mark)?)
                .map(|s| vec![s])
        }
        Err(error) => return Err(error),
    };
    shapes
        .iter()
        .zip(raw)
        .map(|(shape, text)| shape.apply(&codec::reply_lines(&text, mark)))
        .collect()
}
pub fn translate_batch_with<'a>(
    caller: &'a dyn BatchCaller,
    system: &'a str,
    header: &'a str,
    items: &'a [String],
    cancel: &'a CancelToken,
    reask: bool,
) -> Pin<Box<dyn Future<Output = Result<Vec<String>, ProviderError>> + Send + 'a>> {
    Box::pin(async move {
        check_cancel(cancel)?;
        if items.is_empty() {
            return Ok(vec![]);
        }
        let shapes = items.iter().map(|t| Shape::of(t)).collect::<Vec<_>>();
        let texts = shapes.iter().map(|s| s.text.clone()).collect::<Vec<_>>();
        let mark = codec::choose_mark(&texts);
        let mark_note = if mark == DEFAULT_MARK {
            String::new()
        } else {
            format!("\nIn this batch the line-break mark is {mark}, not {DEFAULT_MARK}: keep every {mark} where it belongs.")
        };
        let user = format!("{header}{mark_note}\n\n{}", codec::numbered(&texts, mark));
        let result = match caller.call(system, &user, cancel).await {
            Ok(reply) => parsed(&shapes, &reply, mark).map_err(BatchError::Parse),
            Err(error) => Err(BatchError::Provider(error)),
        };
        match result {
            Ok(output) => Ok(output),
            Err(BatchError::Provider(error)) if error.kind != "truncated" || items.len() == 1 => {
                Err(error)
            }
            Err(BatchError::Parse(_)) if items.len() == 1 && shapes[0].line_count() > 1 => {
                let mut lines = items[0].split('\n').map(str::to_owned).collect::<Vec<_>>();
                let todo = lines
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| !s.trim().is_empty())
                    .map(|(i, _)| i)
                    .collect::<Vec<_>>();
                if todo.is_empty() {
                    return Ok(items.to_vec());
                }
                let source = todo.iter().map(|i| lines[*i].clone()).collect::<Vec<_>>();
                let line_header =
                    format!("{header}\nEach number is one source line; do not reflow it.");
                let translated =
                    translate_batch_with(caller, system, &line_header, &source, cancel, true)
                        .await?;
                for (index, text) in todo.into_iter().zip(translated) {
                    lines[index] = text;
                }
                Ok(vec![lines.join("\n")])
            }
            Err(BatchError::Parse(_)) if items.len() == 1 && reask => {
                let header = format!("{header}{REASK}");
                translate_batch_with(caller, system, &header, items, cancel, false).await
            }
            Err(BatchError::Parse(error)) if items.len() == 1 => {
                Err(ProviderError::new("structure", error.to_string(), ""))
            }
            Err(_) => {
                let middle = items.len() / 2;
                let mut out =
                    translate_batch_with(caller, system, header, &items[..middle], cancel, true)
                        .await?;
                out.extend(
                    translate_batch_with(caller, system, header, &items[middle..], cancel, true)
                        .await?,
                );
                Ok(out)
            }
        }
    })
}
pub async fn translate_using(
    runtime: &ProviderRuntime,
    texts: &[String],
    options: &TranslationOptions,
    configs: &[ProviderConfig],
    system: &str,
    cancel: &CancelToken,
    progress: Option<Progress>,
) -> Result<Vec<String>, ProviderError> {
    let caller = RuntimeCaller { runtime, configs };
    translate_texts_with(&caller, texts, options, system, cancel, progress).await
}
pub async fn translate_texts(
    texts: &[String],
    options: &TranslationOptions,
    configs: &[ProviderConfig],
    system: &str,
    cancel: &CancelToken,
    progress: Option<Progress>,
) -> Result<Vec<String>, ProviderError> {
    let runtime = ProviderRuntime::new(options.global_limit, options.per_provider_limit);
    translate_using(&runtime, texts, options, configs, system, cancel, progress).await
}
/// Pure framing + injected transport seam; used to test malformed/split behavior without credentials.
pub async fn translate_texts_with(
    caller: &dyn BatchCaller,
    texts: &[String],
    options: &TranslationOptions,
    system: &str,
    cancel: &CancelToken,
    progress: Option<Progress>,
) -> Result<Vec<String>, ProviderError> {
    check_cancel(cancel)?;
    let plan = codec::DedupPlan::of(texts);
    let mut translated = plan.unique.clone();
    let eligible = plan
        .unique
        .iter()
        .enumerate()
        .filter(|(_, t)| !t.trim().is_empty())
        .map(|(i, _)| i)
        .collect::<Vec<_>>();
    let source = eligible
        .iter()
        .map(|i| plan.unique[*i].clone())
        .collect::<Vec<_>>();
    let batches = codec::batches(&source, options.max_chars.max(500));
    if let Some(progress) = &progress {
        progress(0, batches.len());
    }
    let header = format!(
        "Target language: {}\nTranslate each numbered line.",
        options.target_lang
    );
    for (index, batch) in batches.iter().enumerate() {
        check_cancel(cancel)?;
        let items = batch.iter().map(|i| source[*i].clone()).collect::<Vec<_>>();
        let output = translate_batch_with(caller, system, &header, &items, cancel, true).await?;
        for (i, text) in batch.iter().zip(output) {
            translated[eligible[*i]] = text;
        }
        if let Some(progress) = &progress {
            progress(index + 1, batches.len());
        }
    }
    plan.restore(&translated)
        .map_err(|e| ProviderError::new("structure", e.to_string(), ""))
}
