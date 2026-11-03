/** The calls about chats: the list, starting one, writing to it, its events, and the choices a new chat offers. */
import { get, post } from "../http";
import type { Change } from "../types/board";
import type { Run } from "../types/cards";
import type {
  Agent,
  AvailableModels,
  Chat,
  ChatImage,
  NewChat,
  RunEvent,
  Skill,
} from "../types/chat";

/** How many events one request asks for; a longer chat is read in pieces. */
export const EVENT_PAGE = 500;

/** The chats, newest first, the ones of the floating window included (the Chat page leaves those out itself). */
export function chats(): Promise<Chat[]> {
  return get<Chat[]>("/v1/chats?all=true");
}

export function startChat(chat: NewChat): Promise<Run> {
  return post<Run>("/v1/chats", chat);
}

/** Writes to a chat that has not ended; the agent may be told where the person is with `context`, and the message may
 * carry `pictures` for a model that looks at images. */
export function say(
  chat: number,
  message: string,
  context?: string,
  pictures: ChatImage[] = [],
): Promise<Run> {
  return post<Run>(`/v1/chats/${chat}/messages`, {
    message,
    ...(context ? { context } : {}),
    ...(pictures.length > 0 ? { images: pictures } : {}),
  });
}

/** Writing to a chat that has ended goes on in a new chat that is told the old one; its first message may carry
 * `pictures` for a model that looks at images. */
export function continueChat(
  chat: number,
  message: string,
  pictures: ChatImage[] = [],
): Promise<Run> {
  return post<Run>("/v1/chats", {
    message,
    continues: chat,
    ...(pictures.length > 0 ? { images: pictures } : {}),
  });
}

export function stopRun(run: number): Promise<Run> {
  return post<Run>(`/v1/runs/${run}/stop`, {});
}

export function offerChat(chat: number): Promise<Change> {
  return post<Change>(`/v1/chats/${chat}/offer`, {});
}

/** Changes the model or the reasoning level of a chat that waits; what is not given stays. */
export function changeChatSettings(
  chat: number,
  settings: { model?: string; thinking?: string },
): Promise<Run> {
  return post<Run>(`/v1/chats/${chat}/settings`, settings);
}

/** Names a chat, in place of the title taken from its first message; an empty name gives that title back. */
export function nameChat(chat: number, title: string): Promise<Chat> {
  return post<Chat>(`/v1/chats/${chat}/title`, { title });
}

/** Says that the person has read the chat up to the event `seq`. */
export async function markSeen(chat: number, seq: number): Promise<void> {
  await post<unknown>(`/v1/chats/${chat}/seen`, { seq });
}

/** Every event of a run with a `seq` above `after`, read in pieces of `EVENT_PAGE`. */
export async function runEvents(run: number, after = 0): Promise<RunEvent[]> {
  const events: RunEvent[] = [];
  for (;;) {
    const from = events.at(-1)?.seq ?? after;
    const page = await get<RunEvent[]>(`/v1/runs/${run}/events?after=${from}&limit=${EVENT_PAGE}`);
    events.push(...page);
    if (page.length < EVENT_PAGE) return events;
  }
}

/**
 * One page of a run's events read backwards: the newest `EVENT_PAGE` of them, or the ones below the `seq` the page
 * before it ended at. A chat is opened at its last message, so its first read goes this way; the rest of the log comes
 * when a person scrolls up to it.
 */
export function olderEvents(run: number, below: number | null): Promise<RunEvent[]> {
  const from = below === null ? "tail=true" : `before=${below}`;
  return get<RunEvent[]>(`/v1/runs/${run}/events?${from}&limit=${EVENT_PAGE}`);
}

export function agents(): Promise<Agent[]> {
  return get<Agent[]>("/v1/agents");
}

export function availableModels(): Promise<AvailableModels> {
  return get<AvailableModels>("/v1/models/available");
}

/** The skills; a server with a harness configuration repository always lists the built-in ones. */
export function skills(): Promise<Skill[]> {
  return get<Skill[]>("/v1/skills");
}

/** A person merges an open change, such as the one a chat offered. */
export async function mergeChange(change: number): Promise<void> {
  await post<unknown>(`/v1/changes/${change}/merge`, {});
}
