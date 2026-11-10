//! The test guest: this binary, started with [`GUEST_ARG`], serves the
//! guest protocol with kardia-guest. Its plugin's name and its behaviour in
//! `apply` come from its arguments; its methods let a check make it answer,
//! hang, exit, read a file or report its environment.

use std::collections::BTreeMap;
use std::sync::OnceLock;
use std::time::Duration;

use kardia_guest::{json, Guest, Host, Level, Map, Meta, Request, Result, Value};

/// The first argument that makes this binary the test guest.
pub const GUEST_ARG: &str = "--hanafubuki-test-guest";

static NAME: OnceLock<String> = OnceLock::new();
static MODE: OnceLock<String> = OnceLock::new();

/// Serves the guest named by `args[0]`, in the mode `args[1]`: `run`,
/// `hang-apply` or `fail-apply`.
pub fn serve(args: &[String]) {
    let _ = NAME.set(args.first().cloned().unwrap_or_default());
    let _ = MODE.set(args.get(1).cloned().unwrap_or_default());
    kardia_guest::serve::<TestGuest>();
}

struct TestGuest;

impl Guest for TestGuest {
    type Config = Value;

    fn meta() -> Meta {
        Meta::plugin(NAME.get().cloned().unwrap_or_default()).provide("plugin")
    }

    fn apply(host: &Host, _config: Value, _imports: Map<String, Value>) -> Result<Self> {
        match MODE.get().map(String::as_str) {
            Some("hang-apply") => loop {
                std::thread::sleep(Duration::from_secs(60));
            },
            Some("fail-apply") => return Err("the guest refuses to apply".into()),
            _ => {}
        }
        eprintln!("the test guest applies");
        host.provide("plugin", &json!({}))?;
        Ok(TestGuest)
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
            "hang" => loop {
                std::thread::sleep(Duration::from_secs(60));
            },
            "exit" => std::process::exit(3),
            method => Err(format!("no method {method}").into()),
        }
    }
}
