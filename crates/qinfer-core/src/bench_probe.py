"""Q-Infer code-quality probe.

Executed by `bench::run` as `python3 -c <this file> <model-path>`.
Never imported by the crate (ADR-0001: engines are subprocesses).

What it measures
----------------
The mean negative log-likelihood per token of a **fixed code corpus**, under the
model itself. Lower is better. That single number does two jobs the product needs
and currently fakes:

  1. **Quantisation damage.** The same model at Q8 and Q4 on the same corpus
     gives two comparable numbers. If 4-bit costs nothing on code, the numbers
     are within noise; if it costs something, this resolves it long before
     pass@1 would, because a pass@1 run over 164 HumanEval problems cannot
     detect a 5-point drop (it needs ~1,090 items per arm at 80% power).
  2. **Capability.** A parameter-count prior knows a 27B is bigger than a 3B and
     nothing about which is better at code. Mean NLL on code is a measurement of
     exactly that, and it is comparable across models because the corpus is fixed.

What it deliberately is NOT
---------------------------
Not instruction-following, not agentic ability, not a benchmark score. It scores
next-token prediction on code, which is a proxy and is labelled as one. It is
reported with its corpus hash so two numbers can only be compared when they were
measured on the same text.

The corpus is checked in and must not change between releases: changing it
invalidates every stored score.
"""

import hashlib
import json
import math
import sys
import time

# Fixed corpus: a mix of Rust and Python, representative of the code a local
# assistant would actually be asked about. Deliberately includes formatting,
# error handling, decorators and comprehensions rather than only happy-path
# expressions, because that is where quantisation damage shows up first.
CORPUS = '''fn parse_headers(raw: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in raw.lines() {
        let Some((k, v)) = line.split_once(':') else { continue };
        out.insert(k.trim().to_string(), v.trim().to_string());
    }
    out
}

pub fn resolve_budget(total: u64, pressure: PressureState) -> u64 {
    match pressure {
        PressureState::Pressured { .. } => total / 2,
        PressureState::Relaxed { .. } => total.saturating_sub(total / 20),
        PressureState::Unknown => total / 2,
    }
}

async def fetch_session(client, session_id):
    async with client.get(f"/v1/sessions/{session_id}") as resp:
        if resp.status != 200:
            raise RuntimeError(f"unexpected status {resp.status}")
        return await resp.json()

class Cache:
    """A bounded LRU cache keyed by content hash."""
    def __init__(self, capacity):
        self.capacity = capacity
        self.store = OrderedDict()

    def get(self, key):
        if key not in self.store:
            return None
        self.store.move_to_end(key)
        return self.store[key]

    def put(self, key, value):
        self.store[key] = value
        self.store.move_to_end(key)
        while len(self.store) > self.capacity:
            self.store.popitem(last=False)

def rolling_mean(values, window):
    total = 0.0
    out = []
    for i, v in enumerate(values):
        total += v
        if i >= window:
            total -= values[i - window]
        out.append(total / min(i + 1, window))
    return out

def group_by(items, key):
    result = defaultdict(list)
    for item in items:
        result[key(item)].append(item)
    return {k: sorted(v) for k, v in result.items()}
'''


def main():
    if len(sys.argv) < 2:
        print(json.dumps({"error": "usage: bench_probe.py <model-path-or-repo>"}))
        return

    model_path = sys.argv[1]
    corpus_sha = hashlib.sha256(CORPUS.encode()).hexdigest()[:16]

    try:
        import mlx.core as mx
        from mlx_lm import load
    except Exception as exc:  # noqa: BLE001
        print(json.dumps({
            "corpus_sha": corpus_sha,
            "error": f"{type(exc).__name__}: {exc}",
        }))
        return

    try:
        t0 = time.perf_counter()
        model, tokenizer = load(model_path)
        load_s = time.perf_counter() - t0

        ids = tokenizer.encode(CORPUS)
        if len(ids) < 32:
            print(json.dumps({
                "corpus_sha": corpus_sha,
                "error": f"corpus tokenised to only {len(ids)} tokens",
            }))
            return

        t0 = time.perf_counter()
        logits = model(mx.array([ids[:-1]]))
        # No mx.log_softmax in this MLX; subtract the log-sum-exp explicitly.
        lp = logits.astype(mx.float32)
        lp = lp - mx.logsumexp(lp, axis=-1, keepdims=True)
        targets = mx.array(ids[1:])[None, :, None]
        picked = mx.take_along_axis(lp, targets, axis=-1).reshape(-1)
        mx.eval(picked)
        forward_s = time.perf_counter() - t0

        mean_nll = float(-picked.mean())
        if not (mean_nll > 0.0) or mean_nll != mean_nll:
            print(json.dumps({
                "corpus_sha": corpus_sha,
                "error": f"model produced a non-positive mean nll ({mean_nll})",
            }))
            return

        print(json.dumps({
            "corpus_sha": corpus_sha,
            "tokens": len(ids),
            "mean_nll": round(mean_nll, 6),
            "perplexity": round(math.exp(mean_nll), 4),
            "load_s": round(load_s, 3),
            "forward_s": round(forward_s, 3),
            "dtype": str(mx.default_device()),
        }))
    except Exception as exc:  # noqa: BLE001
        print(json.dumps({
            "corpus_sha": corpus_sha,
            "error": f"{type(exc).__name__}: {exc}",
        }))


if __name__ == "__main__":
    main()
