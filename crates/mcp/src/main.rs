use anyhow::Result;
use serde_json::{Value, json};
use std::io::{self, BufRead, Write};
fn main() -> Result<()> {
    for line in io::stdin().lock().lines() {
        let line = line?;
        let req: Value = serde_json::from_str(&line)?;
        let id = req.get("id").cloned().unwrap_or(Value::Null);
        let method = req.get("method").and_then(Value::as_str).unwrap_or("");
        let result = match method {
            "initialize" => {
                json!({"protocolVersion":"2025-03-26","capabilities":{"tools":{}},"serverInfo":{"name":"slop-livin","version":env!("CARGO_PKG_VERSION")}})
            }
            "tools/list" => {
                json!({"tools":[{"name":"pressure","description":"Ask whether the volume is under declared pressure","inputSchema":{"type":"object"}},{"name":"changed_since","description":"Ask which facts changed","inputSchema":{"type":"object"}},{"name":"candidates","description":"List fact-bearing candidate units; never a verdict","inputSchema":{"type":"object"}},{"name":"verify","description":"Report observed free-space change","inputSchema":{"type":"object"}}]})
            }
            "tools/call" => {
                json!({"content":[{"type":"text","text":serde_json::to_string(&json!({"state":"awaiting-authorization","cause":"human grant required","next_step":"review facts and create a grant outside the index"}))?}]})
            }
            _ => json!({"state":"unsupported","cause":format!("unknown method {method}")}),
        };
        writeln!(
            io::stdout(),
            "{}",
            json!({"jsonrpc":"2.0","id":id,"result":result})
        )?;
        io::stdout().flush()?;
    }
    Ok(())
}
