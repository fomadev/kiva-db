// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

use std::io::{self, BufRead, Read};
use std::net::{TcpListener, TcpStream};
use std::sync::{atomic::Ordering, Arc};
use std::time::Duration;

use crate::error::{KivaError, Result};
use crate::exec::Out;
use crate::instance::Instance;
use crate::native_admission::Class;
use crate::session::Session;

pub(crate) const WAIT: Duration = Duration::from_secs(2);
pub(crate) const REQUEST_MAX: usize = 2 * 1024 * 1024;
pub(crate) const REPLY_MAX: usize = 4 * 1024 * 1024;

pub(crate) fn start(
    inst: &Arc<Instance>,
    bind: &str,
    port: u16,
    name: &str,
    tls_refusal: &'static [u8],
    busy: &'static [u8],
    serve: fn(&Arc<Instance>, TcpStream),
) -> Result<u16> {
    if !inst.hierarchical_tcp {
        return Err(KivaError::Denied("managed adapters require a hierarchical instance".into()));
    }
    let listener = TcpListener::bind((bind, port))?;
    let address = listener.local_addr()?;
    if !address.ip().is_loopback() && (!inst.cfg().get_bool("tls", "required") || inst.tls().is_none()) {
        return Err(KivaError::Denied("managed adapters require TLS on non-loopback bindings".into()));
    }
    let instance = inst.clone();
    std::thread::Builder::new().name(format!("kiva-managed-{name}")).spawn(move || {
        for conn in listener.incoming() {
            if instance.shutting_down.load(Ordering::Acquire) {
                break;
            }
            let Ok(mut stream) = conn else { continue };
            let (slot, allowed) = crate::server::ClientSlot::claim(&instance);
            if !allowed {
                use std::io::Write;
                let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
                let _ = stream.write_all(busy);
                continue;
            }
            let _ = std::thread::Builder::new().name("kiva-managed-adapter".into()).spawn(move || {
                if let Some((stream, _)) = crate::tls::accept_managed(&slot.0, stream, &|_| tls_refusal.to_vec()) {
                    let _ = stream.set_nodelay(true);
                    serve(&slot.0, stream);
                }
            });
        }
    })?;
    Ok(address.port())
}

/// One bounded CRLF-delimited line, with its delimiter removed.
pub(crate) fn line(reader: &mut impl BufRead, max: usize) -> io::Result<Option<Vec<u8>>> {
    let mut bytes = Vec::new();
    let count = reader.take((max + 1) as u64).read_until(b'\n', &mut bytes)?;
    if count == 0 {
        return Ok(None);
    }
    if count > max || !bytes.ends_with(b"\r\n") {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "line is too large or unterminated"));
    }
    bytes.truncate(count - 2);
    Ok(Some(bytes))
}

pub(crate) fn session(inst: &Arc<Instance>) -> Session {
    let mut session = Session::new_native(inst.clone(), false, None);
    session.adapter_value_limit = Some(REQUEST_MAX);
    session
}

pub(crate) fn login(sess: &mut Session, user: &str, password: &str) -> Result<()> {
    let inst = sess.inst.clone();
    let _permit = inst.native_admission.enter_bounded(Class::Authentication, WAIT)?;
    sess.login(user, password)?;
    Ok(())
}

pub(crate) fn name(value: &str) -> Result<&str> {
    if !crate::auth::valid_name_part(value) {
        return Err(KivaError::Invalid("invalid resource name".into()));
    }
    Ok(value)
}

pub(crate) fn select(sess: &mut Session, path: &str) -> Result<()> {
    let names: Vec<_> = path.split('.').collect();
    if names.len() != 3 || names.iter().any(|part| name(part).is_err()) {
        return Err(KivaError::Invalid("expected workspace.project.collection".into()));
    }
    command(sess, &format!("use collection \"{path}\"")).map(|_| ())
}

/// Used only with fixed verbs and individually validated resource names.
pub(crate) fn command(sess: &mut Session, source: &str) -> Result<Out> {
    let tokens = crate::lexer::tokenize(source).map_err(KivaError::Invalid)?;
    let mut out = Out::default();
    sess.hierarchy_command(&tokens[0].text, &tokens[1..], &mut out)?;
    Ok(out)
}

/// Decreases the adapter's result budget by the wire size of an element.
pub(crate) fn charge_json(remaining: &mut usize, value: &serde_json::Value) -> Result<()> {
    struct Budget<'a>(&'a mut usize);
    impl io::Write for Budget<'_> {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            *self.0 = self.0.checked_sub(bytes.len()).ok_or_else(|| io::Error::other("result budget"))?;
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(Budget(remaining), value)
        .map_err(|_| KivaError::Other("ERR_ADAPTER_LIMIT: result exceeds managed adapter limit".into()))
}
