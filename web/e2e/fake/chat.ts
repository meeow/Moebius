/**
 * The chat routes of the fake server (docs/reference/desktop-api.md, "POST /v1/chats" and the routes next to it) and the
 * run events of a chat as Pi-Bolt's RPC mode and the server make them: `said`, `agent_start`, `message_update` with
 * `text_delta` and `thinking_delta`, `tool_check`, `tool_execution_start` and `_end`, `message_end` with the
 * provider's error, `agent_end`, `agent_settled` and `moebius_error`. A test plays the agent by hand
 * (`delta`, `toolCall`, `providerError`, `settle`), or leaves it to `autoAnswer`, which answers every message with one
 * sentence. Every event is stored under its `seq`, answered by `GET /v1/runs/{id}/events` and sent on the event socket
 * as `{"kind": "run_event", "data": {"run", "seq", "kind", "data"}}`, as the server does.
 */
import type { IncomingMessage, ServerResponse } from "node:http";

export interface StoredEvent {
  seq: number;
  kind: string;
  data: Record<string, unknown>;
  at: number;
}

export interface FakeChatRecord {
  id: number;
  agent: string;
  status: "starting" | "running" | "waiting" | "succeeded" | "failed" | "stopped";
  repository_id: number | null;
  harness_repository: boolean;
  started_at: number;
  ended_at: number | null;
  title: string;
  answered: number;
  seen: number;
  changed_files: number;
  asks: boolean;
  origin: "chat" | "assistant" | "automation";
  model: string | null;
  thinking: string | null;
}

export interface Agent {
  name: string;
  model: string;
  protocol: "openai" | "anthropic";
  thinking?: string;
  harness: "pi-bolt" | "claude-code";
  built_in: boolean;
}

export interface FakeChat {
  /** Answers a chat route and returns true, or returns false for any other path. */
  handle(request: IncomingMessage, response: ServerResponse, path: string): Promise<boolean>;
  /** The chats, newest first. */
  readonly chats: FakeChatRecord[];
  /** The bodies of `POST /v1/chats`, as received. */
  readonly started: Array<Record<string, unknown>>;
  /** The bodies of `POST /v1/chats/{id}/messages`, as received. */
  readonly said: Array<{ chat: number; body: Record<string, unknown> }>;
  /** The `seq` of each `POST /v1/chats/{id}/seen`, as received. */
  readonly seen: Array<{ chat: number; seq: number }>;
  /** Whether a message is answered by the fake itself; a test that plays the agent turns it off. */
  autoAnswer: boolean;
  /** Makes a chat that is already there, with events if given. */
  seed(
    chat: Partial<FakeChatRecord> & { events?: Array<[string, Record<string, unknown>]> },
  ): number;
  /** Stores an event of the run and sends it on the event socket; returns its `seq`. */
  emit(run: number, kind: string, data?: Record<string, unknown>): number;
  agentStart(run: number): void;
  /** One streamed piece of the agent's text. */
  delta(run: number, text: string): void;
  thinking(run: number, text: string): void;
  /** A tool call the check allowed, started and ended. */
  toolCall(
    run: number,
    call: {
      id: string;
      tool: string;
      args: Record<string, unknown>;
      output: string;
      isError?: boolean;
    },
  ): void;
  /** A provider that answered with an error: Pi ends its message with `stopReason: "error"`. */
  providerError(run: number, message: string): void;
  /** The agent's turn ends: the chat waits for the next message. */
  settle(run: number, answer?: string): void;
}

const NOW = () => Math.floor(Date.now() / 1000);

function readBody(request: IncomingMessage): Promise<string> {
  return new Promise((done) => {
    const chunks: Buffer[] = [];
    request.on("data", (chunk: Buffer) => chunks.push(chunk));
    request.on("end", () => done(Buffer.concat(chunks).toString("utf8")));
  });
}

/** Answers the request and returns true, which `handle` says for a route it answered. */
function reply(response: ServerResponse, status: number, body: unknown): true {
  if (typeof body === "string") {
    response.writeHead(status, { "content-type": "text/plain" });
    response.end(body);
    return true;
  }
  response.writeHead(status, { "content-type": "application/json" });
  response.end(JSON.stringify(body));
  return true;
}

/** The agents of a server set up with `moebius init`: the built-in ones have no model, the Coder has one. */
const AGENTS: Agent[] = [
  { name: "Moebius", model: "", protocol: "openai", harness: "pi-bolt", built_in: true },
  { name: "Grilling-partner", model: "", protocol: "openai", harness: "pi-bolt", built_in: true },
  {
    name: "Coder",
    model: "fake-coder",
    protocol: "openai",
    thinking: "medium",
    harness: "pi-bolt",
    built_in: false,
  },
];

const MODELS = {
  models: [
    { id: "fake-coder", protocol: "openai", upstream: "http://127.0.0.1:1" },
    { id: "fake-small", protocol: "openai", upstream: "http://127.0.0.1:1" },
  ],
  errors: [],
  classes: [{ name: "strong", models: ["fake-coder"], protocols: ["openai"] }],
  decision_models: [],
};

const LEVELS = ["off", "minimal", "low", "medium", "high", "xhigh", "max"];

export function createFakeChat(
  broadcast: (event: { kind: string; data: unknown }) => void,
): FakeChat {
  const chats: FakeChatRecord[] = [];
  const events = new Map<number, StoredEvent[]>();
  const started: Array<Record<string, unknown>> = [];
  const said: Array<{ chat: number; body: Record<string, unknown> }> = [];
  const seen: Array<{ chat: number; seq: number }> = [];
  let nextId = 1;

  const find = (id: number) => chats.find((chat) => chat.id === id);

  function run(chat: FakeChatRecord) {
    return {
      id: chat.id,
      card_id: null,
      agent: chat.agent,
      harness: "pi-bolt",
      purpose: "chat",
      status: chat.status,
      worktree: null,
      started_at: chat.started_at,
      ended_at: chat.ended_at,
      summary: null,
      repository_id: chat.repository_id,
      harness_repository: chat.harness_repository,
      origin: chat.origin,
      runner: null,
    };
  }

  const fake: FakeChat = {
    chats,
    started,
    said,
    seen,
    autoAnswer: true,

    emit(id, kind, data = {}) {
      const list = events.get(id) ?? [];
      events.set(id, list);
      const seq = (list.at(-1)?.seq ?? 0) + 1;
      list.push({ seq, kind, data, at: NOW() });
      const chat = find(id);
      if (chat && (kind === "agent_settled" || kind === "moebius_error")) {
        chat.answered = Math.max(chat.answered, seq);
      }
      if (chat && kind === "agent_settled" && data.aborted !== true) chat.status = "waiting";
      if (chat && kind === "agent_start") chat.status = "running";
      broadcast({ kind: "run_event", data: { run: id, seq, kind, data } });
      return seq;
    },

    agentStart(id) {
      fake.emit(id, "agent_start", { type: "agent_start" });
    },

    delta(id, text) {
      fake.emit(id, "message_update", {
        type: "message_update",
        assistantMessageEvent: { type: "text_delta", contentIndex: 0, delta: text },
      });
    },

    thinking(id, text) {
      fake.emit(id, "message_update", {
        type: "message_update",
        assistantMessageEvent: { type: "thinking_delta", contentIndex: 0, delta: text },
      });
    },

    toolCall(id, call) {
      fake.emit(id, "tool_check", {
        tool: call.tool,
        tool_call_id: call.id,
        decision: "allow",
        reason: "no rule is broken",
      });
      fake.emit(id, "tool_execution_start", {
        type: "tool_execution_start",
        toolCallId: call.id,
        toolName: call.tool,
        args: call.args,
      });
      fake.emit(id, "tool_execution_end", {
        type: "tool_execution_end",
        toolCallId: call.id,
        toolName: call.tool,
        isError: call.isError ?? false,
        durationMs: 12,
        result: { content: [{ type: "text", text: call.output }] },
      });
    },

    providerError(id, message) {
      const failed = { role: "assistant", content: [], stopReason: "error", errorMessage: message };
      fake.emit(id, "message_start", { type: "message_start", message: failed });
      fake.emit(id, "message_end", { type: "message_end", message: failed });
      fake.emit(id, "agent_end", { type: "agent_end", messages: [failed] });
      fake.emit(id, "agent_settled", { type: "agent_settled", aborted: false });
    },

    settle(id, answer) {
      const messages = answer
        ? [{ role: "assistant", content: [{ type: "text", text: answer }], stopReason: "stop" }]
        : [];
      fake.emit(id, "agent_end", { type: "agent_end", messages });
      fake.emit(id, "agent_settled", { type: "agent_settled", aborted: false });
    },

    seed(chat) {
      const id = nextId++;
      const { events: history, ...fields } = chat;
      chats.unshift({
        id,
        agent: "Moebius",
        status: "waiting",
        repository_id: null,
        harness_repository: false,
        started_at: NOW(),
        ended_at: null,
        title: "",
        answered: 0,
        seen: 0,
        changed_files: 0,
        asks: false,
        origin: "chat",
        model: null,
        thinking: null,
        ...fields,
      });
      for (const [kind, data] of history ?? []) fake.emit(id, kind, data);
      return id;
    },

    async handle(request, response, path) {
      const method = request.method ?? "GET";
      const url = new URL(request.url ?? "/", "http://x");
      const one =
        /^\/v1\/(?:chats|runs)\/(\d+)(\/(?:events|messages|seen|settings|stop|title))?$/.exec(path);

      if (path === "/v1/agents" && method === "GET") return reply(response, 200, AGENTS);
      if (path === "/v1/models/available" && method === "GET") {
        return reply(response, 200, MODELS);
      }
      if (path === "/v1/skills" && method === "GET") return reply(response, 200, []);

      if (path === "/v1/chats" && method === "GET") {
        const all = url.searchParams.get("all") === "true";
        return reply(
          response,
          200,
          chats.filter((chat) => all || chat.origin !== "assistant"),
        );
      }

      if (path === "/v1/chats" && method === "POST") {
        const body = JSON.parse((await readBody(request)) || "{}") as Record<string, unknown>;
        started.push(body);
        const agent = typeof body.agent === "string" ? body.agent : "Moebius";
        if (!AGENTS.some((each) => each.name === agent)) {
          return reply(response, 422, `no such agent: ${agent}`);
        }
        if (typeof body.message === "string" && body.message.trim() === "") {
          return reply(response, 422, "the message is blank");
        }
        if (typeof body.model === "string" && body.model.trim() === "") {
          return reply(response, 422, "the model is blank");
        }
        if (typeof body.thinking === "string" && !LEVELS.includes(body.thinking)) {
          return reply(response, 422, "thinking is not one of the levels");
        }
        if (agent === "Grilling-partner" && typeof body.repository !== "number") {
          return reply(response, 422, "a grilling needs a repository");
        }
        const message = typeof body.message === "string" ? body.message : null;
        const id = fake.seed({
          agent,
          status: message === null ? "waiting" : "running",
          title: message ? message.split("\n")[0]?.slice(0, 80) : "",
          repository_id: typeof body.repository === "number" ? body.repository : null,
          harness_repository: body.harness === true,
          origin: (body.origin as FakeChatRecord["origin"] | undefined) ?? "chat",
          model: typeof body.model === "string" ? body.model : null,
          thinking: typeof body.thinking === "string" ? body.thinking : null,
        });
        const chat = find(id) as FakeChatRecord;
        if (message !== null) {
          fake.emit(id, "said", { message });
          answer(id, message);
        }
        return reply(response, 200, run(chat));
      }

      if (one && method === "GET" && one[2] === "/events") {
        const id = Number(one[1]);
        if (!find(id)) return reply(response, 404, "no such run");
        const after = Number(url.searchParams.get("after") ?? 0);
        const limit = Number(url.searchParams.get("limit") ?? 500);
        const page = (events.get(id) ?? []).filter((event) => event.seq > after).slice(0, limit);
        return reply(response, 200, page);
      }

      if (one && method === "POST") {
        const id = Number(one[1]);
        const chat = find(id);
        if (!chat) return reply(response, 404, "no such run");
        const body = JSON.parse((await readBody(request)) || "{}") as Record<string, unknown>;
        switch (one[2]) {
          case "/messages": {
            said.push({ chat: id, body });
            const message = typeof body.message === "string" ? body.message : "";
            if (message.trim() === "") return reply(response, 422, "the message is blank");
            if (chat.status === "stopped") {
              // A chat whose agent went away is taken up again by the message, on the record it already has.
              chat.status = "running";
              chat.ended_at = null;
              broadcast({ kind: "run_started", data: { run: id, card: null } });
            } else if (!["starting", "running", "waiting"].includes(chat.status)) {
              return reply(response, 422, "the chat has ended");
            }
            chat.status = "running";
            fake.emit(id, "said", { message });
            answer(id, message);
            return reply(response, 200, run(chat));
          }
          case "/seen":
            seen.push({ chat: id, seq: Number(body.seq) });
            chat.seen = Math.max(chat.seen, Number(body.seq));
            return reply(response, 200, {});
          case "/settings":
            if (chat.status !== "waiting") return reply(response, 409, "the agent is working");
            if (typeof body.model === "string") chat.model = body.model;
            if (typeof body.thinking === "string") chat.thinking = body.thinking;
            return reply(response, 200, run(chat));
          case "/title": {
            const named = (typeof body.title === "string" ? body.title : "").trim().slice(0, 80);
            chat.title = named === "" ? firstLine(id) : named;
            return reply(response, 200, chat);
          }
          case "/stop": {
            const answered = run(chat);
            chat.status = "stopped";
            chat.ended_at = NOW();
            broadcast({ kind: "run_ended", data: { run: id, status: "stopped" } });
            return reply(response, 200, answered);
          }
          default:
            return false;
        }
      }
      return false;
    },
  };

  /** The title a chat is given by its first message, which an empty `POST /v1/chats/{id}/title` gives back. */
  function firstLine(id: number): string {
    const first = (events.get(id) ?? []).find((event) => event.kind === "said");
    const message = first?.data.message;
    return typeof message === "string" ? (message.split("\n")[0] ?? "").slice(0, 80) : "";
  }

  /** The fake's own answer to a message: one sentence, then the chat waits. */
  function answer(id: number, message: string): void {
    if (!fake.autoAnswer) return;
    setTimeout(() => {
      fake.agentStart(id);
      fake.delta(id, `You said: ${message}`);
      fake.settle(id, `You said: ${message}`);
    }, 20);
  }

  return fake;
}
