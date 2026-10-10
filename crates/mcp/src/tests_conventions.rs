use crate::{Backend, Headless, Server, tool_definitions};
use serde_json::{Value, json};

fn rpc(s: &mut Server, method: &str, params: Value) -> Value {
    s.handle(json!({"jsonrpc":"2.0","id":1,"method":method,"params":params})).unwrap()
}
fn call(s: &mut Server, name: &str, arguments: Value) -> Value {
    let r = rpc(s, "tools/call", json!({"name":name,"arguments":arguments}));
    assert!(r.get("error").is_none(), "{r}");
    r["result"].clone()
}
fn payload(r: &Value) -> Value {
    serde_json::from_str(r["content"][0]["text"].as_str().unwrap()).unwrap()
}

#[test]
fn conventions_catalog_and_strict_keys() {
    let mut s = Server::new(Box::new(Headless::with_document()));
    let tools = tool_definitions();
    for name in ["command_list", "command_run", "command_batch", "doc_inspect", "render_preview", "ui_inspect", "ui_screenshot"] {
        assert!(tools.iter().any(|t| t["name"] == name), "missing {name}");
    }
    for t in tools {
        for hint in ["readOnlyHint", "destructiveHint", "idempotentHint", "openWorldHint"] {
            assert!(t["annotations"][hint].is_boolean(), "{t}");
        }
        let r = rpc(&mut s, "tools/call", json!({"name":t["name"],"arguments":{"typo":1}}));
        assert_eq!(r["error"]["code"], -32602, "{r}");
    }
    assert_eq!(rpc(&mut s, "ping", json!({}))["result"], json!({}));
}

#[test]
fn conventions_commands_batch_and_bounded_preview() {
    let mut s = Server::new(Box::new(Headless::with_document()));
    let list = call(&mut s, "command_list", json!({"filter":"shape.rectangle","enabled_only":true}));
    assert_eq!(list["isError"], false, "{list}");
    assert!(payload(&list).as_array().unwrap().iter().any(|c| c["id"] == "shape.rectangle"));
    let all = payload(&call(&mut s, "command_list", json!({"filter":"edit.undo"})));
    assert_eq!(all.as_array().unwrap().len(), 1);
    assert_eq!(all[0]["enabled"], false);
    let enabled = payload(&call(&mut s, "command_list", json!({"filter":"edit.undo","enabled_only":true})));
    assert!(enabled.as_array().unwrap().is_empty());
    let steps = json!([{"id":"shape.rectangle","params":{"x":10,"y":20,"width":30,"height":40}}, {"id":"no.such.command"}, {"id":"shape.rectangle","params":{"x":0,"y":0,"width":20,"height":20}}]);
    for (stop, total) in [(true, 2), (false, 3)] {
        let r = call(&mut s, "command_batch", json!({"steps":steps,"stop_on_error":stop}));
        assert_eq!(r["isError"], true, "{r}");
        let p = payload(&r);
        assert_eq!(p["results"].as_array().unwrap().len(), total);
        assert_eq!(p["failed"], 1);
        assert_eq!(p["completed"], total - 1);
    }
    let before = call(&mut s, "doc_inspect", json!({}));
    let r = call(&mut s, "render_preview", json!({"max_side":64}));
    assert_eq!(r["isError"], false, "{r}");
    let png = vectorcraft_format::base64_decode(r["content"][0]["data"].as_str().unwrap()).unwrap();
    let img = image::load_from_memory(&png).unwrap();
    assert!(img.width() <= 64 && img.height() <= 64);
    assert_eq!(before, call(&mut s, "doc_inspect", json!({})));
    for max in [json!(0), json!(4097), json!("64")] {
        assert_eq!(call(&mut s, "render_preview", json!({"max_side":max}))["isError"], true);
    }
    for name in ["ui_inspect", "ui_screenshot"] {
        assert_eq!(call(&mut s, name, json!({}))["isError"], true);
    }
}

struct Panics;
impl Backend for Panics {
    fn call(&mut self, _: &str, _: Value) -> Result<Value, String> {
        panic!("backend bug")
    }
    fn has_ui(&self) -> bool {
        false
    }
    fn describe(&self) -> String {
        "test".into()
    }
}
#[test]
fn conventions_tool_panics_are_tool_errors() {
    let mut s = Server::new(Box::new(Panics));
    let r = call(&mut s, "command_list", json!({}));
    assert_eq!(r["isError"], true);
    assert!(r["content"][0]["text"].as_str().unwrap().contains("backend bug"));
    assert_eq!(rpc(&mut s, "ping", json!({}))["result"], json!({}));
}

#[test]
fn conventions_resources_use_only_the_current_protocol() {
    let mut s = Server::new(Box::new(Headless::with_document()));
    rpc(&mut s, "initialize", json!({"protocolVersion":"2026-07-28"}));
    for (method, extra) in [
        ("tools/list", json!({})),
        ("resources/list", json!({})),
        ("resources/templates/list", json!({})),
        ("prompts/list", json!({})),
        ("resources/read", json!({"uri":"vectorcraft://document"})),
        ("resources/read", json!({"uri":"vectorcraft://commands"})),
    ] {
        let current = rpc(&mut s, method, extra.clone());
        assert!(current["result"].is_object(), "{current}");
        for key in ["resultType", "ttlMs", "cacheScope"] {
            assert!(current["result"].get(key).is_none());
        }
        let mut params = extra;
        params["_meta"] = json!({"io.modelcontextprotocol/protocolVersion":"2026-07-28"});
        assert_eq!(rpc(&mut s, method, params)["result"], current["result"]);
    }
    let r = rpc(&mut s, "resources/list", json!({}));
    assert!(r["result"]["resources"].as_array().unwrap().iter().any(|r| r["uri"] == "vectorcraft://commands"));
    let commands = rpc(&mut s, "resources/read", json!({"uri":"vectorcraft://commands"}));
    let commands: Value = serde_json::from_str(commands["result"]["contents"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(commands, payload(&call(&mut s, "command_list", json!({}))));
}

#[test]
fn removed_tool_names_are_not_advertised_or_executed() {
    let mut s = Server::new(Box::new(Headless::with_document()));
    for name in ["list_commands", "run_command", "inspect_document", "inspect_ui"] {
        assert!(!tool_definitions().iter().any(|t| t["name"] == name));
        let r = call(&mut s, name, json!({}));
        assert_eq!(r["isError"], true);
        assert!(r["content"][0]["text"].as_str().unwrap().contains("unknown tool"));
    }
}
