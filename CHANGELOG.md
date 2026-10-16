# Changelog

Every version of KivaDB, newest first. Version numbers follow [Semantic Versioning](https://semver.org): MAJOR for incompatible changes, MINOR for new features, PATCH for compatible improvements and fixes.

## 1.0.1 - 2026-10-16

### Improved

* `kivadb start` now shows every listener it started, not only the TCP port:

  ```text
  KivaDB started (pid 13052).
    TCP port : 2911
    REST API : http://127.0.0.1:8080
    Redis    : port 6379
  ```

  When the REST API or the Redis port cannot start (another program already uses the port), the reason is shown on those lines instead, and KivaDB runs without it.

### Internal

* The test suites read the version from `Cargo.toml` instead of repeating it.

## 1.0.0 - 2026-10-15

First release: typed key-value and document database in one executable, with secondary indexes and queries, vector search, transactions, workspaces, users and roles, compaction, compression, replication with failover, Lua scripts and triggers, change notifications, REST API, Redis protocol, Prometheus metrics, Docker image and the Node.js driver. See [DOCUMENTATION.md](DOCUMENTATION.md).
