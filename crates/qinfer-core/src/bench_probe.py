"""Q-Infer code-quality probe.

Executed by `bench::run` as `python3 -c <this file> <model-path> <mode>`.
Never imported by the crate (ADR-0001: engines are subprocesses).

Two modes, two corpora, two hashes
----------------------------------
`raw` scores a block of code as **continuation**: the model sees code and is asked
to keep going. `instruction` applies the model's own chat template to a set of
coding requests and scores only the **assistant span**, which is what a coding
assistant is actually asked to do.

They are different measurements and the corpus hash keeps them apart, so a raw
score can never be silently compared against an instruction score.

`instruction` exists because `raw` is off-distribution: every model in the
catalogue is instruction-tuned, and scoring them on raw continuation measures them
on a task they were not trained for. The first version of this probe only had
`raw`, and it ranked a 30B coder below a 3B general model. Whether that ordering
survives contact with the chat template is the question `instruction` answers.

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

Both corpora are checked in and must not change between releases: changing one
invalidates every stored score keyed to it.
"""

import hashlib
import json
import math
import sys
import time

# Coding requests with reference completions, scored as an assistant turn under
# each model's own chat template. Deliberately weighted towards the shapes a local
# coding assistant is asked for -- a function from a spec, error handling, an
# async call, a data structure, a test -- rather than towards trivia.
INSTRUCTION = [
    {
        "user": "Write a Rust function that returns the nth Fibonacci number.",
        "assistant": "fn fib(n: u64) -> u64 {\n    let (mut a, mut b) = (0, 1);\n    for _ in 0..n {\n        let t = a + b;\n        a = b;\n        b = t;\n    }\n    a\n}",
    },
    {
        "user": "Write a Python function that reads a JSON file and returns a dict, raising a clear error if the file is missing.",
        "assistant": "def load_json(path):\n    try:\n        with open(path) as f:\n            return json.load(f)\n    except FileNotFoundError as exc:\n        raise RuntimeError(f\"missing config: {path}\") from exc",
    },
    {
        "user": "Write a Rust function that splits a string on the first ':' and returns None if there is no colon.",
        "assistant": "fn split_header(line: &str) -> Option<(&str, &str)> {\n    let (key, value) = line.split_once(':')?;\n    Some((key.trim(), value.trim()))\n}",
    },
    {
        "user": "Write a Python decorator that retries a function up to three times on exception.",
        "assistant": "def retry(fn):\n    @functools.wraps(fn)\n    def wrapper(*args, **kwargs):\n        for attempt in range(3):\n            try:\n                return fn(*args, **kwargs)\n            except Exception:\n                if attempt == 2:\n                    raise\n    return wrapper",
    },
    {
        "user": "Write a Rust test that asserts a parsing function rejects an empty string.",
        "assistant": "#[test]\nfn rejects_empty_input() {\n    let err = parse(\"\").unwrap_err();\n    assert!(matches!(err, ParseError::Empty));\n}",
    },
    {
        "user": "Write a Python function that groups a list of dicts by a key and returns a dict of lists.",
        "assistant": "def group_by(items, key):\n    out = {}\n    for item in items:\n        out.setdefault(item[key], []).append(item)\n    return out",
    },
    {
        "user": "Write a Rust function that counts the words in a string.",
        "assistant": "fn word_count(text: &str) -> usize {\n    text.split_whitespace().count()\n}",
    },
    {
        "user": "Write a Python async function that fetches a URL and returns the response body as text.",
        "assistant": "async def fetch_text(client, url):\n    async with client.get(url) as resp:\n        resp.raise_for_status()\n        return await resp.text()",
    },
]

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


def corpus_sha_for(mode):
    """One hash per (mode, corpus). A raw score can never be compared to an
    instruction score, because their hashes differ by construction."""
    if mode == "instruction":
        payload = json.dumps(INSTRUCTION, sort_keys=True)
    else:
        payload = CORPUS
    return hashlib.sha256(payload.encode()).hexdigest()[:16]


def _nll_at(mx, lp_row, token):
    """-log p(token) from a row of log-probabilities."""
    return float(-lp_row[token])


def score_instruction(mx, model, tokenizer):
    """Score only the assistant span, under each model's own chat template.

    The span is derived by tokenising the completion text on its own, so the
    bytes being divided by are exactly the bytes being predicted -- the template's
    trailing end token contributes NLL but no text, and counting it would inflate
    the per-byte figure by an amount that varies with the template.
    """
    total_nll = 0.0
    total_tokens = 0
    total_bytes = 0
    skipped = 0

    for sample in INSTRUCTION:
        user = [{"role": "user", "content": sample["user"]}]
        convo = user + [{"role": "assistant", "content": sample["assistant"]}]

        full = tokenizer.apply_chat_template(convo, tokenize=True, add_generation_prompt=False)
        prompt = tokenizer.apply_chat_template(user, tokenize=True, add_generation_prompt=True)
        # If the template does not render the prompt as a strict prefix, the span
        # cannot be located and the sample must be skipped rather than guessed at.
        if full[: len(prompt)] != prompt:
            skipped += 1
            continue

        comp = tokenizer.encode(sample["assistant"], add_special_tokens=False)
        if len(comp) == 0:
            skipped += 1
            continue
        start = len(prompt)
        end = start + len(comp)
        if end > len(full):
            skipped += 1
            continue

        ids = list(full)
        logits = model(mx.array([ids[:-1]]))
        lp = logits.astype(mx.float32)
        lp = lp - mx.logsumexp(lp, axis=-1, keepdims=True)
        mx.eval(lp)
        row = lp[0]

        # Token at index i is predicted by the logits at i-1.
        for i in range(start, end):
            total_nll += _nll_at(mx, row[i - 1], ids[i])
        total_tokens += end - start
        total_bytes += len(sample["assistant"].encode("utf-8"))

    return total_nll, total_tokens, total_bytes, skipped


def main():
    if len(sys.argv) < 2:
        print(json.dumps({"error": "usage: bench_probe.py <model-path-or-repo> [raw|instruction]"}))
        return

    model_path = sys.argv[1]
    mode = sys.argv[2] if len(sys.argv) > 2 else "raw"
    corpus_sha = corpus_sha_for(mode)

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

        if mode == "instruction":
            t0 = time.perf_counter()
            total_nll, ntokens, nbytes, skipped = score_instruction(mx, model, tokenizer)
            forward_s = time.perf_counter() - t0
            if ntokens < 32 or nbytes == 0:
                print(json.dumps({
                    "corpus_sha": corpus_sha,
                    "error": f"scored only {ntokens} assistant tokens over {nbytes} bytes "
                             f"({skipped} sample(s) skipped) -- the chat template is not usable here",
                }))
                return
            mean_nll = total_nll / ntokens
            extra = {"skipped_samples": skipped, "mode": "instruction"}
        else:
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
            ntokens = len(ids)
            total_nll = mean_nll * ntokens
            nbytes = len(CORPUS.encode("utf-8"))
            extra = {"mode": "raw"}

        if not (mean_nll > 0.0) or mean_nll != mean_nll:
            print(json.dumps({
                "corpus_sha": corpus_sha,
                "error": f"model produced a non-positive mean nll ({mean_nll})",
            }))
            return

        # Per-token NLL is NOT comparable across models: the same text tokenises
        # to a different number of tokens under a different tokeniser, and a
        # coarser tokeniser makes each token carry more bytes and therefore more
        # surprise. Measured on the reference machine: Qwen2.5 scored this corpus
        # in 422 tokens and Qwen3 in 464. Normalising by the corpus's byte length
        # removes the tokeniser from the comparison, which is the only way two
        # models' scores mean the same thing.
        nll_per_byte = total_nll / nbytes

        payload = {
            "corpus_sha": corpus_sha,
            "bytes": nbytes,
            "tokens": ntokens,
            "mean_nll": round(mean_nll, 6),
            "nll_per_byte": round(nll_per_byte, 6),
            "perplexity": round(math.exp(mean_nll), 4),
            "load_s": round(load_s, 3),
            "forward_s": round(forward_s, 3),
            "dtype": str(mx.default_device()),
        }
        payload.update(extra)
        print(json.dumps(payload))
    except Exception as exc:  # noqa: BLE001
        print(json.dumps({
            "corpus_sha": corpus_sha,
            "error": f"{type(exc).__name__}: {exc}",
        }))


if __name__ == "__main__":
    main()
