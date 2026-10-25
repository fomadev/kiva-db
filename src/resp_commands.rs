// Copyright (c) 2026 Fordi / FomaDev.
// Licensed under FomaDev Public License.
// See the LICENSE file in the project root for full license information.

//! Which Redis commands KivaDB answers, and how it refuses the others.
//!
//! Every command goes through [`refuse`] before it runs or is queued in a
//! `MULTI`: a command that is not in [`SUPPORTED`] answers the error Redis
//! gives for an unknown command, so that client libraries recognize it. For
//! the families KivaDB leaves out on purpose (blocking commands, streams,
//! `WATCH`, cluster...), the message also says why and what to use instead.

use super::Reply;

/// Every command the dispatcher runs, sorted. A command missing here is
/// refused before it reaches the dispatcher.
pub(super) const SUPPORTED: &[&str] = &[
    "APPEND",
    "AUTH",
    "CLIENT",
    "COMMAND",
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
    "GETSET",
    "HDEL",
    "HELLO",
    "HEXISTS",
    "HGET",
    "HGETALL",
    "HINCRBY",
    "HKEYS",
    "HLEN",
    "HMGET",
    "HMSET",
    "HSET",
    "HSETNX",
    "HVALS",
    "INCR",
    "INCRBY",
    "INCRBYFLOAT",
    "INFO",
    "KEYS",
    "KIVA.API",
    "KIVA.SIMILAR",
    "LINDEX",
    "LLEN",
    "LPOP",
    "LPUSH",
    "LRANGE",
    "MGET",
    "MSET",
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
    "RPOP",
    "RPUSH",
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
    "SUNION",
    "SUNIONSTORE",
    "TIME",
    "TTL",
    "TYPE",
    "UNLINK",
    "UNSUBSCRIBE",
    "UNWATCH",
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
    Blocking,
    Streams,
    Watch,
    Cluster,
    Functions,
    Replication,
    Modules,
}

impl Family {
    /// Why the family is missing, and what to do instead.
    fn reason(self) -> &'static str {
        match self {
            Family::Blocking => {
                "KivaDB does not support blocking commands: a connection never waits for data. Use the \
                 non-blocking command (LPOP, RPOP, ZPOPMIN, ZPOPMAX) and retry, or SUBSCRIBE to a channel \
                 the producer PUBLISHes to"
            }
            Family::Streams => {
                "KivaDB does not support streams. Use a list as a queue (RPUSH, then LPOP), or PUBLISH and \
                 SUBSCRIBE for live messages"
            }
            Family::Watch => {
                "KivaDB does not support WATCH: MULTI ... EXEC runs as a KivaDB transaction, and EXEC answers \
                 nil when another client changed the same keys meanwhile"
            }
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
        "BLPOP" | "BRPOP" | "BRPOPLPUSH" | "BLMOVE" | "BLMPOP" | "BZPOPMIN" | "BZPOPMAX" | "BZMPOP" | "WAIT"
        | "WAITAOF" => Family::Blocking,
        "WATCH" => Family::Watch,
        "CLUSTER" | "READONLY" | "READWRITE" | "ASKING" | "SPUBLISH" | "SSUBSCRIBE" | "SUNSUBSCRIBE" => Family::Cluster,
        "FUNCTION" | "FCALL" | "FCALL_RO" => Family::Functions,
        "REPLICAOF" | "SLAVEOF" | "SYNC" | "PSYNC" | "REPLCONF" | "FAILOVER" => Family::Replication,
        "MODULE" => Family::Modules,
        n if STREAMS.contains(&n) => Family::Streams,
        _ => return None,
    })
}

const STREAMS: &[&str] = &[
    "XADD",
    "XACK",
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
];

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
        let e = text(refuse("BLPOP", &args(&["blpop", "jobs", "0"])).unwrap());
        assert!(e.starts_with("ERR unknown command 'blpop', with args beginning with: 'jobs' '0' (KivaDB"), "{e}");
        assert!(e.contains("LPOP"), "{e}");
        assert_eq!(family("XADD"), Some(Family::Streams));
        assert_eq!(family("XYZ"), None);
        assert_eq!(family("CLUSTER"), Some(Family::Cluster));
        // A plain unknown command gets the Redis message alone.
        assert_eq!(text(unknown(&args(&["NOPE"]))), "ERR unknown command 'NOPE', with args beginning with: ");
    }
}
