// Adapting the server's OpenAI-style request bodies to WebLLM, and cleaning
// its replies. Pure functions, unit-tested.

type Body = Record<string, unknown> & {
  messages: { role: string; content: unknown }[];
  response_format?: { type?: string; json_schema?: { schema?: unknown } };
};

/** A request body WebLLM accepts: its JSON-schema form, no thinking, no streaming. */
export function toWebLlm(body: Body, modelId: string): Record<string, unknown> {
  const out: Record<string, unknown> = {
    messages: body.messages.map((m) => ({ role: m.role, content: m.content ?? '' })),
    model: modelId,
    stream: false,
    extra_body: { enable_thinking: false }
  };
  if (typeof body.max_tokens === 'number') out.max_tokens = Math.min(body.max_tokens, 2048);
  const schema = body.response_format?.json_schema?.schema;
  if (schema) out.response_format = { type: 'json_object', schema: JSON.stringify(schema) };
  return out;
}

/** Drop any `<think>…</think>` block a hybrid-reasoning model emitted. */
export function stripThinking(text: string): string {
  return text.replace(/<think>[\s\S]*?<\/think>/g, '').trimStart();
}
