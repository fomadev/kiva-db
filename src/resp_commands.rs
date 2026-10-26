// Copyright (c) 2026 Fordi / FomaDev.
// Licensed under FomaDev Public License.
// See the LICENSE file in the project root for full license information.

//! Which Redis commands KivaDB answers, and how it refuses the others.
//!
//! Every command goes through [`refuse`] before it runs or is queued in a
//! `MULTI`: a command that is not in [`SUPPORTED`] answers the error Redis
//! gives for an unknown command, so that client libraries recognize it. For
//! the families KivaDB leaves out on purpose (cluster, functions, modules...),
//! the message also says why and what to use instead.

use super::Reply;

/// Every command the dispatcher runs, sorted. A command missing here is
/// refused before it reaches the dispatcher.
pub(super) const SUPPORTED: &[&str] = &[
    "APPEND",
    "AUTH",
    "BLMOVE",
    "BLMPOP",
    "BLPOP",
    "BRPOP",
    "BRPOPLPUSH",
    "BZMPOP",
    "BZPOPMAX",
    "BZPOPMIN",
    "CLIENT",
    "COMMAND",
    "COPY",
    "DBSIZE",
    "DECR",
    "DECRBY",
    "DEL",
    "DISCARD",
    "ECHO",
    "EVAL",
    "EVALSHA",
    "EVALSHA_RO",
    "EVAL_RO",
    "EXEC",
    "EXISTS",
    "EXPIRE",
    "EXPIREAT",
    "EXPIRETIME",
    "FLUSHALL",
    "FLUSHDB",
    "GET",
    "GETDEL",
    "GETEX",
    "GETRANGE",
    "GETSET",
    "HDEL",
    "HELLO",
    "HEXISTS",
    "HGET",
    "HGETALL",
    "HINCRBY",
    "HINCRBYFLOAT",
    "HKEYS",
    "HLEN",
    "HMGET",
    "HMSET",
    "HRANDFIELD",
    "HSCAN",
    "HSET",
    "HSETNX",
    "HSTRLEN",
    "HVALS",
    "INCR",
    "INCRBY",
    "INCRBYFLOAT",
    "INFO",
    "JSON.ARRAPPEND",
    "JSON.ARRINSERT",
    "JSON.ARRLEN",
    "JSON.DEL",
    "JSON.FORGET",
    "JSON.GET",
    "JSON.MGET",
    "JSON.NUMINCRBY",
    "JSON.NUMMULTBY",
    "JSON.OBJKEYS",
    "JSON.OBJLEN",
    "JSON.SET",
    "JSON.STRLEN",
    "JSON.TYPE",
    "KEYS",
    "KIVA.API",
    "KIVA.SIMILAR",
    "LINDEX",
    "LINSERT",
    "LLEN",
    "LMOVE",
    "LMPOP",
    "LPOP",
    "LPOS",
    "LPUSH",
    "LPUSHX",
    "LRANGE",
    "LREM",
    "LSET",
    "LTRIM",
    "MGET",
    "MSET",
    "MSETNX",
    "MULTI",
    "PERSIST",
    "PEXPIRE",
    "PEXPIREAT",
    "PEXPIRETIME",
    "PING",
    "PSETEX",
    "PSUBSCRIBE",
    "PTTL",
    "PUBLISH",
    "PUBSUB",
    "PUNSUBSCRIBE",
    "QUIT",
    "RENAME",
    "RENAMENX",
    "RPOP",
    "RPOPLPUSH",
    "RPUSH",
    "RPUSHX",
    "SADD",
    "SCAN",
    "SCARD",
    "SCRIPT",
    "SDIFF",
    "SDIFFSTORE",
    "SELECT",
    "SET",
    "SETEX",
    "SETNX",
    "SETRANGE",
    "SINTER",
    "SINTERCARD",
    "SINTERSTORE",
    "SISMEMBER",
    "SMEMBERS",
    "SMISMEMBER",
    "SMOVE",
    "SPOP",
    "SRANDMEMBER",
    "SREM",
    "SSCAN",
    "STRLEN",
    "SUBSCRIBE",
    "SUBSTR",
    "SUNION",
    "SUNIONSTORE",
    "TIME",
    "TOUCH",
    "TTL",
    "TYPE",
    "UNLINK",
    "UNSUBSCRIBE",
    "UNWATCH",
    "WAIT",
    "WATCH",
    "XACK",
    "XADD",
    "XAUTOCLAIM",
    "XCLAIM",
    "XDEL",
    "XGROUP",
    "XINFO",
    "XLEN",
    "XPENDING",
    "XRANGE",
    "XREAD",
    "XREADGROUP",
    "XREVRANGE",
    "XSETID",
    "XTRIM",
    "ZADD",
    "ZCARD",
    "ZCOUNT",
    "ZDIFF",
    "ZDIFFSTORE",
    "ZINCRBY",
    "ZINTER",
    "ZINTERCARD",
    "ZINTERSTORE",
    "ZLEXCOUNT",
    "ZMSCORE",
    "ZPOPMAX",
    "ZPOPMIN",
    "ZRANDMEMBER",
    "ZRANGE",
    "ZRANGEBYLEX",
    "ZRANGEBYSCORE",
    "ZRANGESTORE",
    "ZRANK",
    "ZREM",
    "ZREMRANGEBYLEX",
    "ZREMRANGEBYRANK",
    "ZREMRANGEBYSCORE",
    "ZREVRANGE",
    "ZREVRANGEBYLEX",
    "ZREVRANGEBYSCORE",
    "ZREVRANK",
    "ZSCAN",
    "ZSCORE",
    "ZUNION",
    "ZUNIONSTORE",
];

/// Redis features KivaDB does not have, by design.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Family {
    Cluster,
    Functions,
    Replication,
    Modules,
}

impl Family {
    /// Why the family is missing, and what to do instead.
    fn reason(self) -> &'static str {
        match self {
            Family::Cluster => {
                "KivaDB is not a Redis Cluster: one server holds all of a workspace's data, so connect to it as \
                 a standalone Redis"
            }
            Family::Functions => "KivaDB does not support Redis functions: send the Lua code with EVAL or EVALSHA",
            Family::Replication => {
                "KivaDB replicates with its own protocol: use 'replica start' in the KivaDB shell instead"
            }
            Family::Modules => {
                "KivaDB does not load Redis modules: JSON documents and vector search (KIVA.SIMILAR) are built in"
            }
        }
    }
}

/// The family of a command KivaDB leaves out on purpose; `None` for a
/// supported command or one that is simply unknown.
pub(super) fn family(name: &str) -> Option<Family> {
    Some(match name {
        "CLUSTER" | "READONLY" | "READWRITE" | "ASKING" | "SPUBLISH" | "SSUBSCRIBE" | "SUNSUBSCRIBE" => Family::Cluster,
        "FUNCTION" | "FCALL" | "FCALL_RO" => Family::Functions,
        "REPLICAOF" | "SLAVEOF" | "SYNC" | "PSYNC" | "REPLCONF" | "FAILOVER" => Family::Replication,
        "MODULE" => Family::Modules,
        _ => return None,
    })
}

pub(super) fn is_supported(name: &str) -> bool {
    SUPPORTED.binary_search(&name).is_ok()
}

/// The error for a command KivaDB does not run, in the words Redis uses
/// (`ERR unknown command '<name>', with args beginning with: ...`), followed
/// by the reason when the command belongs to a family left out on purpose.
pub(super) fn unknown(args: &[Vec<u8>]) -> Reply {
    let shown = String::from_utf8_lossy(&args[0]);
    let mut msg = format!("unknown command '{shown}', with args beginning with: ");
    // Like Redis, the arguments are quoted until 128 characters are shown.
    for a in &args[1..] {
        if msg.len() > 128 {
            break;
        }
        msg.push_str(&format!("'{}' ", String::from_utf8_lossy(a)));
    }
    if let Some(f) = family(&shown.to_ascii_uppercase()) {
        msg.push_str(&format!("({}.)", f.reason()));
    }
    Reply::err(msg)
}

/// `Some(error)` when `name` (upper case) is not a command KivaDB runs.
pub(super) fn refuse(name: &str, args: &[Vec<u8>]) -> Option<Reply> {
    (!is_supported(name)).then(|| unknown(args))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(r: Reply) -> String {
        match r {
            Reply::Error(e) => e,
            _ => panic!("not an error"),
        }
    }

    fn args(a: &[&str]) -> Vec<Vec<u8>> {
        a.iter().map(|s| s.as_bytes().to_vec()).collect()
    }

    #[test]
    fn the_list_is_sorted_and_without_duplicates() {
        assert!(SUPPORTED.windows(2).all(|w| w[0] < w[1]), "keep SUPPORTED sorted: binary search relies on it");
        for name in SUPPORTED {
            assert_eq!(family(name), None, "{name} is both supported and refused");
        }
    }

    #[test]
    fn refused_families_explain_themselves() {
        assert!(refuse("GET", &args(&["GET", "k"])).is_none());
        assert!(refuse("BLPOP", &args(&["BLPOP", "q", "0"])).is_none());
        let e = text(refuse("FCALL", &args(&["fcall", "f", "0"])).unwrap());
        assert!(e.starts_with("ERR unknown command 'fcall', with args beginning with: 'f' '0' (KivaDB"), "{e}");
        assert!(e.contains("EVAL"), "{e}");
        assert_eq!(family("XADD"), None);
        assert_eq!(family("XYZ"), None);
        assert_eq!(family("CLUSTER"), Some(Family::Cluster));
        // A plain unknown command gets the Redis message alone.
        assert_eq!(text(unknown(&args(&["NOPE"]))), "ERR unknown command 'NOPE', with args beginning with: ");
    }
}
