// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Embedded Lua scripting.
//!
//! Scripts run in a sandbox (no file system, no OS access, no dynamic code
//! loading), under a time and memory limit, and atomically: every write a
//! script makes is buffered in a transaction that is committed only if the
//! script finishes without error.

use crate::error::{KivaError, Result};
use crate::session::Session;
use crate::value::{fmt_f64, Value};
use mlua::{HookTriggers, Lua, LuaOptions, MultiValue, StdLib, Table, Value as LVal, VmState};
use serde_json::Value as Json;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

#[derive(Default, Debug)]
pub struct ScriptOutput {
    /// Lines produced by `print(...)` (and the line trace in debug mode).
    pub printed: Vec<String>,
    /// The script's return value, rendered as text.
    pub result: Option<String>,
}

fn lerr(e: impl std::fmt::Display) -> mlua::Error {
    mlua::Error::RuntimeError(e.to_string())
}

fn json_to_lua(lua: &Lua, j: &Json) -> mlua::Result<LVal> {
    Ok(match j {
        Json::Null => LVal::Nil,
        Json::Bool(b) => LVal::Boolean(*b),
        Json::Number(n) => match n.as_i64() {
            Some(i) => LVal::Integer(i),
            None if n.is_f64() => LVal::Number(n.as_f64().unwrap_or(0.0)),
            // An integer beyond a Lua integer: a float when it holds it exactly, else its text.
            None => match crate::num::exact_f64(&n.to_string()) {
                Some(f) => LVal::Number(f),
                None => LVal::String(lua.create_string(n.to_string())?),
            },
        },
        Json::String(s) => LVal::String(lua.create_string(s)?),
        Json::Array(a) => {
            let t = lua.create_table()?;
            for (i, v) in a.iter().enumerate() {
                t.raw_set(i + 1, json_to_lua(lua, v)?)?;
            }
            LVal::Table(t)
        }
        Json::Object(m) => {
            let t = lua.create_table()?;
            for (k, v) in m {
                t.raw_set(k.as_str(), json_to_lua(lua, v)?)?;
            }
            LVal::Table(t)
        }
    })
}

fn value_to_lua(lua: &Lua, v: &Value) -> mlua::Result<LVal> {
    Ok(match v {
        Value::Str(s) => LVal::String(lua.create_string(s)?),
        // A number no Lua number holds exactly comes as its text, not rounded.
        Value::Num(n) => match (n.parse::<i64>(), crate::num::exact_f64(n)) {
            (Ok(i), _) => LVal::Integer(i),
            (Err(_), Some(f)) => LVal::Number(f),
            (Err(_), None) => LVal::String(lua.create_string(n)?),
        },
        Value::Bool(b) => LVal::Boolean(*b),
        Value::Null => LVal::Nil,
        Value::Doc(j) => json_to_lua(lua, j)?,
        Value::Bytes(b) => LVal::String(lua.create_string(b)?),
        Value::Date(_) | Value::DateTime(_) | Value::Time(_) => {
            LVal::String(lua.create_string(v.to_json().as_str().unwrap_or(""))?)
        }
    })
}

fn lua_to_json(v: &LVal, depth: usize) -> mlua::Result<Json> {
    if depth > 64 {
        return Err(lerr("table is nested too deeply"));
    }
    Ok(match v {
        LVal::Nil => Json::Null,
        LVal::Boolean(b) => Json::Bool(*b),
        LVal::Integer(i) => Json::Number((*i).into()),
        LVal::Number(f) => serde_json::Number::from_f64(*f).map(Json::Number).unwrap_or(Json::Null),
        LVal::String(s) => Json::String(s.to_string_lossy().to_string()),
        LVal::Table(t) => table_to_json(t, depth)?,
        other => return Err(lerr(format!("cannot store a Lua {}", other.type_name()))),
    })
}

fn table_to_json(t: &Table, depth: usize) -> mlua::Result<Json> {
    let len = t.raw_len();
    let mut total = 0usize;
    for pair in t.pairs::<LVal, LVal>() {
        pair?;
        total += 1;
    }
    if len > 0 && total == len {
        let mut a = Vec::with_capacity(len);
        for i in 1..=len {
            a.push(lua_to_json(&t.raw_get::<LVal>(i)?, depth + 1)?);
        }
        return Ok(Json::Array(a));
    }
    let mut m = serde_json::Map::new();
    for pair in t.pairs::<LVal, LVal>() {
        let (k, v) = pair?;
        let key = match &k {
            LVal::String(s) => s.to_string_lossy().to_string(),
            LVal::Integer(i) => i.to_string(),
            LVal::Number(f) => fmt_f64(*f),
            other => return Err(lerr(format!("table keys must be strings or numbers, not {}", other.type_name()))),
        };
        m.insert(key, lua_to_json(&v, depth + 1)?);
    }
    Ok(Json::Object(m))
}

fn lua_to_value(v: &LVal) -> mlua::Result<Value> {
    Ok(match v {
        LVal::Nil => Value::Null,
        LVal::Boolean(b) => Value::Bool(*b),
        LVal::Integer(i) => Value::Num(i.to_string()),
        LVal::Number(f) if f.is_finite() => Value::Num(fmt_f64(*f)),
        LVal::Number(_) => return Err(lerr("cannot store NaN or infinity")),
        LVal::String(s) => match std::str::from_utf8(&s.as_bytes()) {
            Ok(text) => Value::Str(text.to_string()),
            Err(_) => Value::Bytes(s.as_bytes().to_vec()),
        },
        LVal::Table(t) => Value::Doc(table_to_json(t, 0)?),
        other => return Err(lerr(format!("cannot store a Lua {}", other.type_name()))),
    })
}

fn render(v: &LVal) -> String {
    match v {
        LVal::Nil => "(nil)".to_string(),
        LVal::Boolean(b) => b.to_string(),
        LVal::Integer(i) => i.to_string(),
        LVal::Number(f) => fmt_f64(*f),
        LVal::String(s) => s.to_string_lossy().to_string(),
        LVal::Table(t) => table_to_json(t, 0).map(|j| j.to_string()).unwrap_or_else(|_| "<table>".into()),
        other => format!("<{}>", other.type_name()),
    }
}

/// A Lua interpreter with no access to files, the operating system or code
/// loading, under the configured memory and time limits. With `trace`, the
/// lines executed are recorded there (`--lua-debug`).
pub(crate) fn sandbox(sess: &Session, trace: Option<Rc<RefCell<Vec<String>>>>) -> Result<Lua> {
    let (timeout_ms, mem_mb) = {
        let c = sess.inst.cfg();
        (c.get_u64("scripting", "timeout_ms"), c.get_u64("scripting", "memory_limit_mb"))
    };
    let lua = Lua::new_with(StdLib::TABLE | StdLib::STRING | StdLib::MATH | StdLib::UTF8, LuaOptions::default())?;
    lua.set_memory_limit((mem_mb as usize).saturating_mul(1024 * 1024))?;
    let globals = lua.globals();
    // The base library is always present: remove everything that can load
    // code or reach outside the sandbox.
    for name in ["dofile", "loadfile", "load", "loadstring", "require", "collectgarbage"] {
        globals.set(name, LVal::Nil)?;
    }
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut triggers = HookTriggers::new().every_nth_instruction(1000);
    if trace.is_some() {
        triggers = triggers.every_line();
    }
    lua.set_hook(triggers, move |_, dbg| {
        if let Some(t) = &trace {
            if dbg.event() == mlua::DebugEvent::Line {
                t.borrow_mut().push(format!("[lua-debug] line {}", dbg.curr_line()));
            }
        }
        if Instant::now() > deadline {
            return Err(lerr(format!("script exceeded the time limit of {timeout_ms} ms")));
        }
        Ok(VmState::Continue)
    });
    Ok(lua)
}

fn run_lua(sess: &mut Session, source: &str, args: &[String]) -> Result<ScriptOutput> {
    let printed: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let lua = sandbox(sess, sess.inst.opts.lua_debug.then(|| printed.clone()))?;
    let globals = lua.globals();

    let cell = RefCell::new(sess);
    let out = printed.clone();
    let result = lua.scope(|scope| {
        let kiva = lua.create_table()?;
        let get = scope.create_function(|lua, key: String| match cell.borrow_mut().kv_get(&key).map_err(lerr)? {
            Some((v, _)) => value_to_lua(lua, &v),
            None => Ok(LVal::Nil),
        })?;
        let set = scope.create_function(|_, (key, value, ttl): (String, LVal, Option<i64>)| {
            crate::exec::validate_key_name(&key).map_err(lerr)?;
            let v = lua_to_value(&value)?;
            let mut s = cell.borrow_mut();
            // Keep the storage format and TTL of an existing key unless a TTL is given.
            let meta = s.kv_get(&key).map_err(lerr)?.map(|(_, m)| m).unwrap_or_default();
            let expires_at = match ttl {
                Some(t) if t > 0 => crate::util::deadline_in(t),
                Some(_) => 0,
                None => meta.expires_at,
            };
            s.kv_put(&key, v, expires_at, meta.msgpack).map_err(lerr)?;
            Ok(true)
        })?;
        let del = scope.create_function(|_, key: String| cell.borrow_mut().kv_del(&key).map_err(lerr))?;
        let has = scope.create_function(|_, key: String| cell.borrow_mut().kv_exists(&key).map_err(lerr))?;
        let type_of = scope.create_function(|_, key: String| {
            Ok(cell.borrow_mut().kv_type(&key).map_err(lerr)?.map(|t| t.name().to_string()))
        })?;
        let scan = scope.create_function(|lua, pattern: Option<String>| {
            let s = cell.borrow();
            s.check(crate::session::Need::Read).map_err(lerr)?;
            let keys = s.engine.keys_matching(pattern.as_deref().unwrap_or("*"), 0);
            let t = lua.create_table()?;
            for (i, (k, _)) in keys.into_iter().enumerate() {
                t.raw_set(i + 1, k)?;
            }
            Ok(t)
        })?;
        // Integers stay exact integers; a fraction on either side makes it a float.
        let incr = scope.create_function(|_, (key, delta): (String, Option<LVal>)| {
            crate::exec::validate_key_name(&key).map_err(lerr)?;
            let mut s = cell.borrow_mut();
            let (cur, meta) = match s.kv_get(&key).map_err(lerr)? {
                Some((Value::Num(n), m)) => (n, m),
                Some(_) => return Err(lerr(format!("key '{key}' is not a number"))),
                None => ("0".to_string(), Default::default()),
            };
            let (text, next) = match (cur.parse::<i64>(), delta.unwrap_or(LVal::Integer(1))) {
                (Ok(a), LVal::Integer(b)) => {
                    let n = a.checked_add(b).ok_or_else(|| lerr(format!("incrementing '{key}' would overflow")))?;
                    (n.to_string(), LVal::Integer(n))
                }
                (_, d) => {
                    let d = match d {
                        LVal::Integer(i) => i as f64,
                        LVal::Number(f) => f,
                        _ => return Err(lerr("the increment must be a number")),
                    };
                    let a = crate::num::exact_f64(&cur)
                        .ok_or_else(|| lerr(format!("'{key}' holds {cur}, which a float cannot hold exactly")))?;
                    let n = a + d;
                    if !n.is_finite() {
                        return Err(lerr("the increment would produce NaN or Infinity"));
                    }
                    (fmt_f64(n), LVal::Number(n))
                }
            };
            s.kv_put(&key, Value::Num(text), meta.expires_at, false).map_err(lerr)?;
            Ok(next)
        })?;
        kiva.set("get", get)?;
        kiva.set("set", set)?;
        kiva.set("del", del)?;
        kiva.set("has", has.clone())?;
        kiva.set("exists", has)?;
        kiva.set("typeof", type_of)?;
        kiva.set("scan", scan.clone())?;
        kiva.set("keys", scan)?;
        kiva.set("incr", incr)?;
        globals.set("kiva", kiva)?;

        let print = scope.create_function(|_, values: MultiValue| {
            let line = values.iter().map(render).collect::<Vec<_>>().join("\t");
            out.borrow_mut().push(line);
            Ok(())
        })?;
        globals.set("print", print)?;
        globals.set("ARGV", lua.create_sequence_from(args.iter().map(|s| s.as_str()))?)?;

        let values = lua.load(source).set_name("script").eval::<MultiValue>()?;
        Ok(if values.is_empty() { None } else { Some(values.iter().map(render).collect::<Vec<_>>().join(", ")) })
    });
    lua.remove_hook();
    let result = result.map_err(|e| {
        // Keep the message readable: drop the Lua traceback.
        let text = e.to_string();
        KivaError::Script(text.split("\nstack traceback").next().unwrap_or(&text).trim().to_string())
    })?;
    let printed = printed.borrow().clone();
    Ok(ScriptOutput { printed, result })
}

/// Runs Lua source atomically.
pub fn eval(sess: &mut Session, source: &str, args: &[String]) -> Result<ScriptOutput> {
    let implicit = sess.txn.is_none();
    if implicit {
        sess.begin()?;
    }
    let res = run_lua(sess, source, args);
    if implicit {
        match &res {
            Ok(_) => {
                sess.commit()?;
            }
            Err(_) => {
                sess.rollback();
            }
        }
    }
    res
}

fn script_key(name: &str) -> String {
    format!("script:{name}")
}

pub fn valid_script_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

pub fn load_source(sess: &Session, name: &str) -> Result<Option<String>> {
    match sess.inst.system.get(&script_key(name))? {
        Some((Value::Str(s), _)) => Ok(Some(s)),
        _ => Ok(None),
    }
}

pub fn store(sess: &Session, name: &str, source: &str) -> Result<()> {
    if !valid_script_name(name) {
        return Err(KivaError::Invalid("script names may only contain letters, digits, '_', '-' and '.'".into()));
    }
    // Refuse scripts that do not even parse.
    Lua::new_with(StdLib::NONE, LuaOptions::default())?
        .load(source)
        .set_name(name)
        .into_function()
        .map_err(|e| KivaError::Script(format!("syntax error: {e}")))?;
    sess.inst.system.put(&script_key(name), &Value::Str(source.to_string()), 0, false)?;
    Ok(())
}

pub fn remove(sess: &Session, name: &str) -> Result<bool> {
    sess.inst.system.delete(&script_key(name))
}

/// `(name, size in bytes)` of every stored script.
pub fn list(sess: &Session) -> Vec<(String, u32)> {
    sess.inst
        .system
        .keys_matching("script:*", 0)
        .into_iter()
        .map(|(k, e)| (k["script:".len()..].to_string(), e.raw_size))
        .collect()
}

pub fn run_stored(sess: &mut Session, name: &str, args: &[String]) -> Result<ScriptOutput> {
    let source =
        load_source(sess, name)?.ok_or_else(|| KivaError::NotFound(format!("script '{name}' does not exist")))?;
    eval(sess, &source, args)
}
