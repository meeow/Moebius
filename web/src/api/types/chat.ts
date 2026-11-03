/** Types of the chats and their run events; they follow docs/reference/desktop-api.md, "Types" and the chat routes. */
import type { RunStatus } from "./board";

/** Where a chat was started: the Chat page, an automation, or the floating window (`assistant`). */
export type ChatOrigin = "chat" | "automation" | "assistant";

/** A chat as `GET /v1/chats` lists it. */
export interface Chat {
  /** The id of its run. */
  id: number;
  agent: string;
  status: RunStatus;
  repository_id: number | null;
  harness_repository: boolean;
  started_at: number;
  ended_at: number | null;
  /** The first line of the first message, at most 80 characters; empty before the first message. */
  title: string;
  /** The `seq` of the last `agent_settled` or `moebius_error`; 0 without one. */
  answered: number;
  /** How far a person has read it; the chat is unread while `answered` is greater. */
  seen: number;
  changed_files: number;
  /** True while the chat waits and the agent's last answer ends in a question. */
  asks: boolean;
  origin: ChatOrigin;
  /** The model the chat was given in place of its agent's; null while it uses its agent's. */
  model: string | null;
  thinking: string | null;
}

/** A picture a chat message carries, as `POST /v1/chats` and `POST /v1/chats/{id}/messages` take it: what the
 * person's clipboard or file was called, which kind of image it is, and its bytes in base64. */
export interface ChatImage {
  name?: string;
  media_type: string;
  data: string;
}

/** The body of `POST /v1/chats`; every field is optional. */
export interface NewChat {
  agent?: string;
  message?: string;
  /** The pictures the first message carries, for a model that looks at images. */
  images?: ChatImage[];
  model?: string;
  thinking?: string;
  repository?: number;
  harness?: boolean;
  continues?: number;
  context?: string;
  origin?: ChatOrigin;
}

/** One stored event of a run, as `GET /v1/runs/{id}/events` has it. */
export interface RunEvent {
  seq: number;
  kind: string;
  data: Record<string, unknown>;
  /** Unix seconds. */
  at: number;
}

/** An agent a chat can be with, as `GET /v1/agents` lists it. */
export interface Agent {
  name: string;
  /** Empty for a built-in agent that has no model of its own or of the Coder. */
  model: string;
  protocol: "openai" | "anthropic";
  thinking?: string;
  token_budget?: number;
  harness: "pi-bolt" | "claude-code";
  built_in: boolean;
  error?: string;
}

export interface AvailableModel {
  id: string;
  protocol: string;
  upstream: string;
  /** Whether the model takes pictures along with its text, which a chat checks before it sends one. */
  images: boolean;
}

export interface AvailableClass {
  name: string;
  models: string[];
  protocols: string[];
  /** True only when every model of the class takes pictures, since a chat cannot know which of them answers. */
  images: boolean;
}

/** What `GET /v1/models/available` says. */
export interface AvailableModels {
  models: AvailableModel[];
  errors: Array<{ upstream: string; message: string }>;
  classes: AvailableClass[];
  decision_models: string[];
}

export interface Skill {
  name: string;
  description: string;
  built_in: boolean;
}

/** The reasoning levels Pi-Bolt takes. */
export const THINKING_LEVELS = ["off", "minimal", "low", "medium", "high", "xhigh", "max"] as const;
