use async_trait::async_trait;
use color_eyre::eyre::eyre;

use crate::ai::tool::Tool;


#[derive(Default)]
pub struct ListTool {}

#[async_trait]
impl Tool for ListTool {
    
    fn name(&self) -> String {
        "ls".to_string()
    }

    fn description(&self) -> Option<String> {
        Some(format!(
            "List directory contents. Returns entries sorted alphabetically, with '/' suffix for directories. 
            Includes dotfiles."
        ))
    }

    fn parameters(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The Directory to list, use . to represent current working directory"
                },
            },
            "required": ["path"]
        }))
    }

    async fn execute(&self, args: serde_json::Value) -> color_eyre::Result<String> {
        let path = args["path"]
            .as_str()
            .ok_or_else(||eyre!("missing parameter: path"))?;

        let mut items = Vec::new();
        let path = std::path::Path::new(path);
        for item in std::fs::read_dir(path)? {
            let item = item?;
            if item.metadata()?.is_dir() {
                items.push(format!("{}/",item.file_name().to_string_lossy()));
            } else {
                items.push(format!("{}",item.file_name().to_string_lossy()));
            }
        }
        let ret = items.join("\n");

        Ok(ret)
    }
}


#[cfg(test)]
mod test {

    use super::*;
    #[test]
    fn test_path() -> color_eyre::Result<()> {
        let path = std::path::Path::new("./src");
        for item in std::fs::read_dir(path)? {
            let item = item?;
            if item.metadata()?.is_dir() {
                println!("{}/",item.file_name().to_string_lossy())
            } else {
                println!("{}",item.file_name().to_string_lossy())
            }
        }
        Ok(())
    }
}