// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! The semantic cache, as an application calling a language model uses it.

mod common;

use common::{fails, instance, ok, TempDir};
use kivadb::semantic_cache::{Lookup, Question};
use kivadb::session::Session;
use serde_json::json;
use std::time::{Duration, Instant};

fn session(dir: &TempDir) -> Session {
    Session::new(instance(dir), true, None)
}

/// Deterministic vectors centred on 0, as embeddings are.
fn vectors(n: usize, dims: usize, seed: u64) -> Vec<Vec<f32>> {
    let mut s = seed;
    (0..n)
        .map(|_| {
            (0..dims)
                .map(|_| {
                    s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                    (s >> 33) as f32 / u32::MAX as f32 - 0.25
                })
                .collect()
        })
        .collect()
}

#[test]
fn questions_answers_and_invalidation_from_the_shell() {
    let dir = TempDir::new("semantic_shell");
    let mut s = session(&dir);
    let out = ok(
        &mut s,
        r#"semantic store faq text "What is the capital of the DRC?" vector [1, 0, 0] answer "Kinshasa" depends "geo:*""#,
    );
    assert_eq!(out, "Stored as faq:1 (expires in 300 s).");

    // Worded differently, nearly the same embedding: the stored answer.
    assert_eq!(ok(&mut s, "semantic lookup faq vector [1, 0.1, 0]"), r#"Hit (similarity 0.9950, faq:1): "Kinshasa""#);
    // The same words, no embedding needed.
    assert!(ok(&mut s, r#"semantic lookup faq text "what is the  capital of the drc?""#)
        .starts_with("Hit (similarity 1.0000"));
    // Another question: a miss, with how close it came.
    assert_eq!(
        ok(&mut s, "semantic lookup faq vector [0.6, 0.8, 0]"),
        "Miss (closest similarity 0.6000, threshold 0.98)"
    );
    assert!(ok(&mut s, "semantic lookup faq vector [0.6, 0.8, 0] threshold 0.5").starts_with("Hit"));
    assert_eq!(ok(&mut s, "semantic lookup other text \"x\""), "Miss (nothing cached for this question)");

    // Data the answer depends on changes: the answer goes.
    ok(&mut s, r#"set geo:cd {"capital": "Kinshasa"}"#);
    assert!(ok(&mut s, "semantic lookup faq vector [1, 0, 0]").starts_with("Miss"));

    // Answers of any type, with their own lifetime.
    ok(&mut s, r#"semantic store faq vector [0, 1, 0] answer {"text": "Goma", "sources": ["doc:7"]} ttl 1"#);
    assert!(ok(&mut s, "semantic lookup faq vector [0, 1, 0]").contains(r#"{"text":"Goma","sources":["doc:7"]}"#));
    std::thread::sleep(Duration::from_millis(2100));
    assert!(ok(&mut s, "semantic lookup faq vector [0, 1, 0]").starts_with("Miss"));

    assert!(ok(&mut s, "semantic stats").starts_with("faq: 0 entries, 4 hits, 3 misses (57% hit rate)"));
    ok(&mut s, r#"semantic store faq text "q" answer 42 ttl 0"#);
    assert_eq!(ok(&mut s, "semantic clear faq"), "Cleared 1 cached answer(s).");
    assert!(fails(&mut s, "semantic lookup faq").contains("text, its vector, or both"));
    assert!(fails(&mut s, "semantic store faq text \"q\"").contains("store needs an answer"));
    assert!(fails(&mut s, "semantic lookup faq vector [1, 0, 0] threshold 2").contains("between 0 and 1"));
    ok(&mut s, r#"semantic store faq vector [1, 0, 0] answer 1"#);
    assert!(fails(&mut s, "semantic lookup faq vector [1, 0]").contains("3 dimensions"));
}

#[test]
fn get_similar_reuses_the_results_of_a_near_identical_query() {
    let dir = TempDir::new("semantic_similar");
    let mut s = session(&dir);
    ok(&mut s, r#"set doc:1 {"title": "Kinshasa", "embedding": [0.9, 0.1, 0.0]}"#);
    ok(&mut s, r#"set doc:2 {"title": "Rust", "embedding": [0.0, 0.2, 0.95]}"#);
    let first = ok(&mut s, "get similar doc:*.embedding to [1, 0, 0] limit 1 return title");
    assert!(first.contains("documents read from disk"), "off by default: {first}");

    ok(&mut s, "config set semantic.cache.enabled true");
    ok(&mut s, "get similar doc:*.embedding to [1, 0, 0] limit 1 return title");
    let again = ok(&mut s, "get similar doc:*.embedding to [1, 0.01, 0] limit 1 return title");
    assert!(again.starts_with(r#"doc:1: 0.99"#) && again.contains("semantic cache"), "{again}");
    // Another limit is another query.
    assert!(!ok(&mut s, "get similar doc:*.embedding to [1, 0, 0] limit 2").contains("semantic cache"));
    // `exact` always computes.
    assert!(!ok(&mut s, "get similar doc:*.embedding to [1, 0, 0] limit 1 exact").contains("semantic cache"));
    // A write under the pattern makes the cached results stale.
    ok(&mut s, r#"set doc:3 {"title": "Lubumbashi", "embedding": [1.0, 0.0, 0.0]}"#);
    let fresh = ok(&mut s, "get similar doc:*.embedding to [1, 0, 0] limit 1 return title");
    assert!(fresh.starts_with("doc:3: 1.0000") && !fresh.contains("semantic cache"), "{fresh}");

    // The structured request says when it was served from the cache.
    let q = json!({ "op": "similar", "target": "doc:*.embedding", "vector": [1, 0.01, 0], "limit": 1 });
    let r = kivadb::api::handle(&mut s, &q).unwrap();
    assert!(r["cached"].as_f64().unwrap() > 0.99, "{r}");
}

#[test]
fn the_structured_request_drives_the_cache() {
    let dir = TempDir::new("semantic_api");
    let mut s = session(&dir);
    let stored = kivadb::api::handle(
        &mut s,
        &json!({ "op": "semantic", "action": "store", "cache": "llm", "text": "Hello?", "vector": [0, 0, 1],
                 "answer": { "reply": "Mbote!" }, "ttl": 60, "depends": ["kb:*"] }),
    )
    .unwrap();
    assert_eq!(stored["id"], "llm:1");
    let hit =
        kivadb::api::handle(&mut s, &json!({ "op": "semantic", "cache": "llm", "vector": [0, 0.05, 1] })).unwrap();
    assert_eq!((hit["hit"].as_bool(), hit["answer"]["reply"].as_str()), (Some(true), Some("Mbote!")));
    let miss = kivadb::api::handle(&mut s, &json!({ "op": "semantic", "cache": "llm", "text": "Bye?" })).unwrap();
    assert_eq!(miss["hit"], false);
}

#[test]
fn a_cache_in_front_of_a_slow_model_saves_most_of_its_calls() {
    let dir = TempDir::new("semantic_bench");
    let inst = instance(&dir);
    let cache = &inst.semantic;
    let dims = 128;
    // 2000 questions answered once by the "model".
    let known = vectors(2000, dims, 1);
    for (i, v) in known.iter().enumerate() {
        let q = Question { text: None, vector: Some(v.clone()) };
        cache.store("default", "llm", &q, json!(format!("answer {i}")), 0, vec![], 10_000).unwrap();
    }
    // Traffic: 400 rewordings of known questions (a little noise on their
    // embedding), 100 questions never seen.
    let noise = vectors(400, dims, 2);
    let fresh = vectors(100, dims, 3);
    // A language model answers in hundreds of milliseconds: counted, not slept.
    let model_cost = Duration::from_millis(300);
    let (mut hits, mut wrong, mut calls) = (0, 0, 0);
    let mut lookup_time = Duration::ZERO;
    let start = Instant::now();
    let traffic = (0..400)
        .map(|i| {
            let src = i * 5;
            let v: Vec<f32> = known[src].iter().zip(&noise[i]).map(|(a, n)| a + 0.03 * n).collect();
            (Some(src), v)
        })
        .chain(fresh.into_iter().map(|v| (None, v)));
    for (source, v) in traffic {
        let q = Question { text: None, vector: Some(v) };
        let t = Instant::now();
        let found = cache.lookup("default", "llm", &q, 0.98).unwrap();
        lookup_time += t.elapsed();
        match found {
            Lookup::Hit { answer, .. } => {
                hits += 1;
                if Some(answer) != source.map(|s| json!(format!("answer {s}"))) {
                    wrong += 1;
                }
            }
            Lookup::Miss { .. } => {
                // Ask the model, then remember its answer.
                calls += 1;
                cache.store("default", "llm", &q, json!("computed"), 0, vec![], 10_000).unwrap();
            }
        }
    }
    let with_cache = start.elapsed() + model_cost * calls;
    let without_cache = model_cost * 500;
    println!(
        "hits {hits}/500, model calls {calls}, wrong answers {wrong}, average lookup {:?}, \
         {with_cache:?} with the cache against {without_cache:?} without",
        lookup_time / 500
    );
    assert_eq!(wrong, 0, "a hit must give the answer of the question it rewords");
    assert!(hits >= 390, "only {hits} of the 400 rewordings were recognised");
    assert_eq!(calls, 500 - hits);
    assert!(with_cache < without_cache * 3 / 10, "{with_cache:?} with the cache, {without_cache:?} without");
}
