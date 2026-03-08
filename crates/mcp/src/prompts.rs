use rmcp::model::{
    GetPromptRequestParams, GetPromptResult, Prompt, PromptArgument, PromptMessage,
    PromptMessageRole,
};

pub const BROWSE_STORAGE_PROMPT: &str = "browse_storage";
pub const SUMMARIZE_FILE_PROMPT: &str = "summarize_file";

pub fn list_prompts() -> Vec<Prompt> {
    vec![
        Prompt::new(
            BROWSE_STORAGE_PROMPT,
            Some("List the Infimount virtual root and then inspect a selected storage path."),
            Some(vec![
                PromptArgument::new("storage_name")
                    .with_description("Optional storage name to navigate into after listing '/'."),
                PromptArgument::new("path")
                    .with_description("Optional absolute path inside the selected storage.")
                    .with_required(false),
            ]),
        ),
        Prompt::new(
            SUMMARIZE_FILE_PROMPT,
            Some("Read a file through MCP and summarize its contents."),
            Some(vec![PromptArgument::new("path")
                .with_description("Absolute file path to summarize.")
                .with_required(true)]),
        ),
    ]
}

pub fn get_prompt(request: GetPromptRequestParams) -> Result<GetPromptResult, String> {
    let arguments = request.arguments.unwrap_or_default();

    match request.name.as_str() {
        BROWSE_STORAGE_PROMPT => {
            let storage_name = string_arg(&arguments, "storage_name");
            let path = string_arg(&arguments, "path")
                .or_else(|| storage_name.as_ref().map(|name| format!("/{name}")));

            let first_step =
                "Call list_dir with {\"path\":\"/\"} to inspect the Infimount virtual root.";
            let second_step = match path {
                Some(path) => format!(
                    "Then call list_dir with {{\"path\":\"{path}\"}} and continue navigating deterministically."
                ),
                None => "Then choose one listed storage and call list_dir on that storage root."
                    .to_string(),
            };

            Ok(GetPromptResult::new(vec![PromptMessage::new_text(
                PromptMessageRole::User,
                format!("{first_step}\n{second_step}"),
            )])
            .with_description(
                "Browse a mounted storage by starting at '/' and descending with list_dir.",
            ))
        }
        SUMMARIZE_FILE_PROMPT => {
            let path = string_arg(&arguments, "path")
                .ok_or_else(|| "prompt 'summarize_file' requires a 'path' argument".to_string())?;

            Ok(GetPromptResult::new(vec![PromptMessage::new_text(
                PromptMessageRole::User,
                format!(
                    "Call read_file with {{\"path\":\"{path}\"}} and summarize the returned content. If the file is binary or truncated, say so explicitly."
                ),
            )])
            .with_description("Read a file through MCP and summarize the result."))
        }
        _ => Err(format!("prompt '{}' is not defined", request.name)),
    }
}

fn string_arg(arguments: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<String> {
    arguments
        .get(key)
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::GetPromptRequestParams;
    use serde_json::json;

    #[test]
    fn prompt_list_is_deterministic() {
        let names = list_prompts()
            .into_iter()
            .map(|prompt| prompt.name)
            .collect::<Vec<_>>();
        assert_eq!(names, vec![BROWSE_STORAGE_PROMPT, SUMMARIZE_FILE_PROMPT]);
    }

    #[test]
    fn summarize_prompt_requires_path() {
        let err = get_prompt(GetPromptRequestParams::new(SUMMARIZE_FILE_PROMPT)).unwrap_err();
        assert!(err.contains("requires a 'path'"));
    }

    #[test]
    fn browse_prompt_includes_virtual_root_instruction() {
        let prompt = get_prompt(
            GetPromptRequestParams::new(BROWSE_STORAGE_PROMPT)
                .with_arguments(rmcp::model::object(json!({"storage_name": "Local"}))),
        )
        .unwrap();

        assert_eq!(prompt.messages.len(), 1);
        let PromptMessage {
            content: rmcp::model::PromptMessageContent::Text { text },
            ..
        } = &prompt.messages[0]
        else {
            panic!("expected text prompt");
        };
        assert!(text.contains("\"path\":\"/\""));
        assert!(text.contains("/Local"));
    }
}
