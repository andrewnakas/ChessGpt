// The coach model running in this browser (WebGPU via WebLLM). When it is
// loaded, this tab holds /api/llm/device open and answers the server's coach
// requests, so the coach costs the site nothing.
import type { DeviceModel } from '$lib/api/types';
import { stripThinking, toWebLlm } from './wire';

type Status = 'off' | 'unsupported' | 'loading' | 'ready' | 'error';

// Minimal slice of the WebLLM engine we use (the module loads lazily).
type Engine = {
  chat: { completions: { create: (req: Record<string, unknown>) => Promise<Record<string, unknown>> } };
  unload: () => Promise<void>;
};

const OPT_IN_KEY = 'chessgpt.device-model';

function readOptIn(): string | null {
  try {
    return localStorage.getItem(OPT_IN_KEY);
  } catch {
    return null;
  }
}

function writeOptIn(id: string | null) {
  try {
    if (id) localStorage.setItem(OPT_IN_KEY, id);
    else localStorage.removeItem(OPT_IN_KEY);
  } catch {
    /* storage blocked: the opt-in just isn't remembered */
  }
}

/** Does this browser have a usable WebGPU adapter? */
export async function webgpuSupported(): Promise<boolean> {
  const gpu = (navigator as Navigator & { gpu?: { requestAdapter: () => Promise<unknown> } }).gpu;
  if (!gpu) return false;
  try {
    return (await gpu.requestAdapter()) != null;
  } catch {
    return false;
  }
}

class DeviceCoach {
  status = $state<Status>('off');
  progress = $state(0);
  detail = $state('');
  modelId = $state<string | null>(null);
  /** Requests answered in this tab (for the settings page). */
  answered = $state(0);

  #engine: Engine | null = null;
  #events: EventSource | null = null;
  #queue: Promise<void> = Promise.resolve();

  get ready() {
    return this.status === 'ready';
  }

  /** Load the model again on page load if the user turned it on before. */
  async resume(model: DeviceModel | null | undefined) {
    if (!model || this.status !== 'off') return;
    if (readOptIn() === model.id) await this.enable(model);
  }

  async enable(model: DeviceModel) {
    if (this.status === 'loading' || this.status === 'ready') return;
    if (!(await webgpuSupported())) {
      this.status = 'unsupported';
      this.detail = 'This browser has no WebGPU. Try a recent Chrome, Edge or Safari on a computer.';
      return;
    }
    this.status = 'loading';
    this.progress = 0;
    this.detail = 'Starting…';
    try {
      const webllm = await import('@mlc-ai/web-llm');
      const appConfig = model.url
        ? {
            model_list: [
              { model: model.url, model_id: model.id, model_lib: model.lib_url ?? '', overrides: { context_window_size: 8192 } }
            ]
          }
        : webllm.prebuiltAppConfig;
      const worker = new Worker(new URL('./worker.ts', import.meta.url), { type: 'module' });
      this.#engine = (await webllm.CreateWebWorkerMLCEngine(
        worker,
        model.id,
        {
          appConfig,
          initProgressCallback: (r: { progress: number; text: string }) => {
            this.progress = r.progress;
            this.detail = r.text;
          }
        },
        { context_window_size: 8192 }
      )) as unknown as Engine;
      this.modelId = model.id;
      writeOptIn(model.id);
      this.#connect(model.id);
      this.status = 'ready';
      this.detail = '';
    } catch (e) {
      this.status = 'error';
      this.detail = e instanceof Error ? e.message : String(e);
      await this.#engine?.unload().catch(() => {});
      this.#engine = null;
    }
  }

  async disable() {
    writeOptIn(null);
    this.#events?.close();
    this.#events = null;
    await this.#engine?.unload().catch(() => {});
    this.#engine = null;
    this.status = 'off';
    this.modelId = null;
  }

  #connect(id: string) {
    this.#events?.close();
    const es = new EventSource(`/api/llm/device?model=${encodeURIComponent(id)}`);
    es.addEventListener('request', (ev) => {
      const req = JSON.parse((ev as MessageEvent).data) as { id: string; body: Record<string, unknown> };
      // One generation at a time; the server queues the rest here.
      this.#queue = this.#queue.then(() => this.#answer(req.id, req.body));
    });
    this.#events = es;
  }

  async #answer(id: string, body: Record<string, unknown>) {
    let reply: unknown;
    try {
      if (!this.#engine || !this.modelId) throw new Error('model not loaded');
      const res = await this.#engine.chat.completions.create(
        toWebLlm(body as Parameters<typeof toWebLlm>[0], this.modelId)
      );
      const choices = (res.choices as { message?: { content?: string } }[]) ?? [];
      for (const c of choices) {
        if (c.message?.content) c.message.content = stripThinking(c.message.content);
      }
      reply = res;
      this.answered += 1;
    } catch (e) {
      reply = { error: e instanceof Error ? e.message : String(e) };
    }
    await fetch(`/api/llm/relay/${encodeURIComponent(id)}`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(reply)
    }).catch(() => {});
  }
}

export const device = new DeviceCoach();

/** The coach works when the server has a model or this tab runs one. */
export function coachAvailable(hasProvider: boolean | undefined): boolean {
  return !!hasProvider || device.ready;
}
