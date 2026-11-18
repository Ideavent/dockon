# AI provider decision

The user chose their own cloud AI provider at `https://ai.ideavent.tr/v1`. After activation, `GET /models` and `POST /chat/completions` were reachable and required a key. The adapter uses the commonly supported Bearer and chat-completions JSON format. Successful authenticated responses still need to be verified with an authorized key.

Keep the provider transport/encoding adapter separate from context collection, consent, redaction, repair-plan validation and the local permission/execution boundary. Provider output is untrusted data; adapters cannot grant themselves tools or execution rights. Native project management remains independent and works offline.

The preview sends only the aggregate diagnostic context, with an explicit button click and a freshness check before transmission. Output is rendered as plain text. The preview currently keeps credentials in memory for the duration of the interaction. Persistent OS-backed secret storage and schema-validated repair plans remain pending.
