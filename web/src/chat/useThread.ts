/**
 * Follows one chat: reads its stored events, then takes the `run_event`s of the stream, and gives the page the
 * conversation's entries. Events are pushed into the conversation as they come and the page is told at most once per
 * animation frame, so a fast answer redraws a few dozen times a second, not once per streamed piece. After the socket
 * connected again the events it missed are read.
 */
import { useEffect, useRef, useState } from "react";
import { runEvents } from "../api/calls/chat";
import type { RunEvent } from "../api/types/chat";
import { useConnection, useServerEvents } from "../live/LiveProvider";
import { Conversation, type Entry } from "./conversation";

export interface Thread {
  entries: Entry[];
  /** The highest `seq` the page holds; 0 when none. */
  lastSeq: number;
  /** Whether the stored events were read. */
  loaded: boolean;
  /** Why the events could not be read, if they could not. */
  error: string | null;
}

const EMPTY: Thread = { entries: [], lastSeq: 0, loaded: false, error: null };

/** The `run_event` of the stream as a stored event, when it is one of chat `id`. */
export function runEventOf(
  id: number,
  event: { kind: string; data: Record<string, unknown> },
): RunEvent | null {
  const { data } = event;
  if (event.kind !== "run_event" || data.run !== id) return null;
  if (typeof data.seq !== "number" || typeof data.kind !== "string") return null;
  const inner =
    typeof data.data === "object" && data.data !== null
      ? (data.data as Record<string, unknown>)
      : {};
  return { seq: data.seq, kind: data.kind, data: inner, at: Math.floor(Date.now() / 1000) };
}

export function useThread(id: number | null): Thread {
  const [thread, setThread] = useState<Thread>(EMPTY);
  const conversation = useRef<{ id: number; value: Conversation; loaded: boolean } | null>(null);
  const frame = useRef<number | null>(null);
  const connection = useConnection();

  const publish = (error: string | null) => {
    const current = conversation.current;
    if (!current) return;
    setThread({
      entries: current.value.entries(),
      lastSeq: current.value.lastSeq,
      loaded: current.loaded,
      error,
    });
  };

  // Reads the chat when it is opened, and what it missed whenever the socket is (again) online.
  // biome-ignore lint/correctness/useExhaustiveDependencies: `publish` only reads refs and sets state
  useEffect(() => {
    if (id === null) {
      conversation.current = null;
      setThread(EMPTY);
      return;
    }
    if (conversation.current?.id !== id) {
      conversation.current = { id, value: new Conversation(id), loaded: false };
      setThread(EMPTY);
    }
    if (connection === "reconnecting") return;
    const current = conversation.current;
    let cancelled = false;
    runEvents(id, current.value.lastSeq)
      .then((events) => {
        if (cancelled || conversation.current !== current) return;
        for (const event of events) current.value.push(event);
        current.loaded = true;
        publish(null);
      })
      .catch((e: unknown) => {
        if (cancelled || conversation.current !== current) return;
        publish(e instanceof Error ? e.message : "cannot read the chat");
      });
    return () => {
      cancelled = true;
    };
  }, [id, connection]);

  useServerEvents((event) => {
    const current = conversation.current;
    if (!current) return;
    const stored = runEventOf(current.id, event);
    if (!stored) return;
    current.value.push(stored);
    if (frame.current !== null) return;
    frame.current = requestAnimationFrame(() => {
      frame.current = null;
      publish(null);
    });
  });

  useEffect(
    () => () => {
      if (frame.current !== null) cancelAnimationFrame(frame.current);
    },
    [],
  );

  return thread;
}
