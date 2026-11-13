# AI provider decision

The user will supply their own cloud AI provider. Do not hardcode a vendor, model, endpoint, authentication mechanism, or assume OpenAI protocol compatibility.

Keep the provider transport/encoding adapter separate from context collection, consent, redaction, repair-plan validation and the local permission/execution boundary. Provider output is untrusted data; adapters cannot grant themselves tools or execution rights. Native project management remains independent and works offline.

Before a live provider integration can be verified, obtain its API contract: endpoint shape, authentication scheme, example request/response, streaming/error semantics and model selection if applicable. Credentials belong in the application's local secret store, not repository configuration or chat messages.
