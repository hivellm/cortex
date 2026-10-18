# Offline classification contract fixture v1

CC0-1.0. Six invented candidates and human-readable category judgments were authored independently of the retrieval corpus. No real user content, credentials or provider output appears here.

`expected_category` states the fixture judgment. `synthetic_response` is deliberately authored provider wire data, not a Jev observation. Four answers match; one lesson is deliberately mislabeled as a decision, and one clear decision receives unknown. Thus mocked accuracy is 4/6, with two unknown answers (one expected, one incorrect). These numbers test honest scoring and must never be presented as measured Jev accuracy.

Each response supplies all six probabilities: selected category 0.75, each other category 0.05, totaling one. Confidence is independently 0.65. The fixture exercises typed probabilities and accounting, not empirical calibration. Thirteen input and five output tokens per response are synthetic accounting values, not billed usage.

Tests use the production Provider HTTP parser and durable queue against a literal loopback server, with an inert fixture credential. No external network or environment credential is used. Separate fault cases alter copies of responses for malformed schema/distribution/model, HTTP 429 and a response deadline; frozen judgments remain unchanged. Successful responses exercise accepted, proposed and observational sources; fault cases exercise an accepted source. Complete source records, including authority and content hashes, must remain unchanged in every tested outcome. Classification results stay observations linked to their source.

This is a deterministic contract evaluation. It does not test live model accuracy, retrieval quality, latency performance, or actual provider costs. Live classification requires a separately authorized opt-in workflow.

`questions.json` pins the production question contract. The Rust test checks it against prepared request bodies. Live preview must verify the entire reconstructed request SHA-256 against the binary-produced prepared hash before sending. Labels and synthetic responses are never request content.
