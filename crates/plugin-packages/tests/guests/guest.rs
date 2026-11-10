//! The test guest: this binary, started with [`GUEST_ARG`], serves the
//! guest protocol with kardia-guest. Its plugin's name and its behaviour in
//! `apply` come from its arguments; its methods let a check make it answer,
//! hang, exit, write a line to stderr, read a file or report its
//! environment, and answer the Agent's contribution calls (`tool`,
//! `context`, `prompt`, the hooks, `observe`) as `contribute` expects.

use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use kardia_guest::{json, Guest, Host, Level, Map, Meta, Request, Result, Value};

/// The first argument that makes this binary the test guest.
pub const GUEST_ARG: &str = "--hanafubuki-test-guest";

static NAME: OnceLock<String> = OnceLock::new();
static MODE: OnceLock<String> = OnceLock::new();
/// The events `observe` was told, in order.
static OBSERVED: Mutex<Vec<Value>> = Mutex::new(Vec::new());

/// Serves the guest named by `args[0]`, in the mode `args[1]`: `run`,
/// `hang-apply`, `fail-apply`, `provide-apply` (it provides a second
/// service), or `hang-contributions` (its context and prompt never answer).
pub fn serve(args: &[String]) {
    let _ = NAME.set(args.first().cloned().unwrap_or_default());
    let _ = MODE.set(args.get(1).cloned().unwrap_or_default());
    kardia_guest::serve::<TestGuest>();
}

struct TestGuest {
    config: Value,
}

fn hangs() -> ! {
    loop {
        std::thread::sleep(Duration::from_secs(60));
    }
}

fn mode() -> &'static str {
    MODE.get().map(String::as_str).unwrap_or_default()
}

impl Guest for TestGuest {
    type Config = Value;

    fn meta() -> Meta {
        Meta::plugin(NAME.get().cloned().unwrap_or_default()).provide("plugin")
    }

    fn apply(host: &Host, config: Value, _imports: Map<String, Value>) -> Result<Self> {
        match mode() {
            "hang-apply" => hangs(),
            "fail-apply" => return Err("the guest refuses to apply".into()),
            "provide-apply" => host.provide("stolen", &json!({}))?,
            _ => {}
        }
        eprintln!("the test guest applies");
        host.provide("plugin", &json!({}))?;
        Ok(TestGuest { config })
    }

    fn call(&mut self, host: &Host, request: Request) -> Result<Value> {
        match request.method.as_str() {
            "echo" => Ok(request.args),
            "pid" => Ok(json!(std::process::id())),
            "env" => Ok(json!(std::env::vars().collect::<BTreeMap<_, _>>())),
            "cwd" => Ok(json!(std::env::current_dir()?)),
            "read" => Ok(json!(std::fs::read_to_string(
                request.args.as_str().unwrap_or_default()
            )?)),
            "write" => {
                std::fs::write(request.args.as_str().unwrap_or_default(), "written")?;
                Ok(Value::Null)
            }
            "speak" => {
                eprintln!("SECRET-STDERR");
                host.log(Level::Error, "SECRET-LOG");
                Ok(Value::Null)
            }
            "stderr" => {
                eprintln!("{}", request.args.as_str().unwrap_or_default());
                Ok(Value::Null)
            }
            "hang" => hangs(),
            "exit" => std::process::exit(3),
            "config" => Ok(self.config.clone()),
            "view" => host.view(request.args.as_str().unwrap_or_default()),
            "invoke" => host.invoke(
                request.args["key"].as_str().unwrap_or_default(),
                request.args["method"].as_str().unwrap_or_default(),
                &Value::Null,
            ),
            "provide" => {
                host.provide(request.args.as_str().unwrap_or_default(), &json!({}))?;
                Ok(Value::Null)
            }
            "observed" => Ok(json!(*OBSERVED.lock().unwrap())),
            method => contribution(method, &request.args),
        }
    }
}

/// The answers to the Agent's contribution calls.
fn contribution(method: &str, args: &Value) -> Result<Value> {
    let hanging = mode() == "hang-contributions";
    match method {
        "tool" => match args["name"].as_str().unwrap_or_default() {
            "echo" => Ok(json!({ "echoed": args["arguments"], "call": args["call"] })),
            "text" => Ok(json!("plain text")),
            "fail" => Err("SECRET-TOOL-FAILURE".into()),
            "hang" => hangs(),
            "exit" => std::process::exit(4),
            name => Err(format!("no tool {name}").into()),
        },
        "context" if hanging => hangs(),
        "context" => match args["kind"].as_str().unwrap_or_default() {
            "empty_board" => Ok(Value::Null),
            "broken_board" => Err("SECRET-CONTEXT-FAILURE".into()),
            kind => Ok(json!(format!("{kind} for session {}", args["sessionId"]))),
        },
        "prompt" if hanging => hangs(),
        "prompt" => Ok(json!(format!("Prompt for step {}.", args["samplingStep"]))),
        "beforeToolUse" => Ok(match args["call"]["toolName"].as_str() {
            Some("forbidden") => json!({ "deny": "the test guest denies it" }),
            _ => Value::Null,
        }),
        "afterToolUse" => Ok(json!({
            "context": [format!("after {}", args["call"]["toolName"].as_str().unwrap_or_default())]
        })),
        "continuation" => {
            let answer = args["candidateFinalAnswer"].as_str().unwrap_or_default();
            Ok(if answer.contains("more") {
                json!({ "continue": ["keep going"] })
            } else if answer.contains("stop") {
                json!("stop")
            } else {
                Value::Null
            })
        }
        "observe" => {
            OBSERVED.lock().unwrap().push(args.clone());
            Ok(Value::Null)
        }
        method => Err(format!("no method {method}").into()),
    }
}
