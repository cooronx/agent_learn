use async_trait::async_trait;
use serde::Deserialize;
use schemars::{JsonSchema, schema_for};
use color_eyre::{Result, eyre::eyre};

use crate::ai::tool::Tool;



#[derive(Default)]
pub struct EditTool;

#[derive(Deserialize,JsonSchema)]
pub struct ReplaceEdit {
    #[schemars(description = "Exact text to replace. Must be unique in the original file")]
    old_text: String,
    #[schemars(description = "Replacement text")]
    new_text: String,
}


#[derive(Deserialize,JsonSchema)]
pub struct EditParameters {
    #[schemars(length(min = 1), description = "Path to the file to edit")]
    path: String,
    #[schemars(
        length(min = 1),
        description = "One or more targeted replacements. Each edit is matched against the original file, not incrementally"
    )]
    edits: Vec<ReplaceEdit>,
}

#[async_trait]
impl Tool for EditTool {
    fn name(&self) ->  String {
        "edit".to_string()
    }

    fn description(&self) -> Option<String> {
        Some(
            "Edit a single file using exact text replacement. Every edits[].oldText must match a \
             unique, non-overlapping part of the original file."
                .to_string(),
        )
    }

    fn parameters(&self) -> Option<serde_json::Value> {
        Some(schema_for!(EditParameters).to_value())
    }

    async fn execute(&self, args: serde_json::Value) -> Result<String> {
        let args: EditParameters = serde_json::from_value(args)?;

        if args.edits.is_empty() {
            return Err(eyre!("edits must not be empty"));
        }

        let path = std::env::current_dir()?.join(&args.path);
        let raw = tokio::fs::read_to_string(&path).await?;

        // 对bom文件做额外处理
        let bom = raw.starts_with('\u{feff}');
        let content = raw.strip_prefix('\u{feff}').unwrap_or(&raw);

        // 将所有的换行统一为 LF
        let crlf = content.contains("\r\n");
        let content = content.replace("\r\n", "\n");


        // 先定位每一个要修改的地方，统一成一个三元组的结构格式（start,count,第i个edit）
        let mut span = Vec::new();
        for (i,edit) in args.edits.iter().enumerate() {
            if edit.old_text.is_empty() {
                return Err(eyre!("edits[{i}].old_text must not be empty"));
            }

            let Some(index) = content.find(&edit.old_text) else {
                return Err(eyre!("edits[{i}].old_text not found in {}",args.path));
            };

            let count = content.matches(&edit.old_text).count();

            if count > 1 {
                return Err(eyre!(
                    "edits[{i}].oldText appears {count} times in {}: provide more context to make it unique",args.path
                ));
            }

            span.push((index,edit.old_text.len(),i));
        }

        // 先排序
        span.sort();
        // 先检查一下有没有重叠的情况
        if span.windows(2).any(|w|w[0].0 + w[0].1 > w[1].0) {
            return Err(eyre!("edits overlap in {}: merge them into one edit",args.path));
        }

        // 我们要从后往前去进行替换
        let mut new_content = content.clone();
        for &(index,len,i) in span.iter().rev() {
            new_content.replace_range(index..index+len, &args.edits[i].new_text);
        }

        // 如果前后完全一样，说明这个ai在乱写
        if new_content == content {
            return Err(eyre!("no changes made to {}",args.path));
        }

        let mut output_content = if crlf {
            new_content.replace("\n", "\r\n")
        } else {
            new_content
        };

        if bom {
            output_content.insert(0, '\u{feff}');
        }
        tokio::fs::write(&path, output_content).await?;
        
        Ok(format!(
            "replaced {} spans in {}",args.edits.len(),args.path
        ))
    }
}

