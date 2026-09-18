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
                json!({"tools":[{"name":"pressure","description":"Ask whether the volume is under declared pressure","inputSchema":{"type":"object"}},{"name":"changed_since","description":"Ask which facts changed","inputSchema":{"type":"object"}},{"name":"candidates","description":"List fact-bearing candidate units; never a verdict","inputSchema":{"type":"object"}},{"name":"propose","description":"Create a frozen plan; never authorizes execution","inputSchema":{"type":"object"}},{"name":"verify","description":"Report observed free-space change","inputSchema":{"type":"object"}},{"name":"execute","description":"Execute only with a human grant","inputSchema":{"type":"object"}}]})
            }
            "tools/call" => {
                let params = req.get("params").cloned().unwrap_or_default();
                let name = params.get("name").and_then(Value::as_str).unwrap_or("");
                let answer = match name {
                    "pressure" => {
                        json!({"state":"question","tool":"pressure","next_step":"compare free space with the declared floor"})
                    }
                    "changed_since" => {
                        json!({"state":"question","tool":"changed_since","next_step":"refresh facts if the horizon was exceeded"})
                    }
                    "candidates" => {
                        json!({"state":"insufficient-evidence","tool":"candidates","candidates":[],"next_step":"scan or refresh before proposing"})
                    }
                    "propose" => {
                        json!({"state":"awaiting-authorization","tool":"propose","plan":"frozen-after-facts","next_step":"ask the human for a scoped grant"})
                    }
                    "verify" => {
                        json!({"state":"question","tool":"verify","next_step":"re-observe free space and residuals after the action"})
                    }
                    "execute" => {
                        json!({"state":"awaiting-authorization","cause":"human grant required","next_step":"review facts and create a grant outside the index"})
                    }
                    _ => json!({"state":"unsupported","cause":"unknown question-shaped tool"}),
                };
                json!({"content":[{"type":"text","text":serde_json::to_string(&answer)?}]})
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
