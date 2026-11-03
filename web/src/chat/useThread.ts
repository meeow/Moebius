/**
 * Follows one chat: reads its stored events, then takes the `run_event`s of the stream, and gives the page the
 * conversation's entries. Events are pushed into the conversation as they come and the page is told at most once per
 * animation frame, so a fast answer redraws a few dozen times a second, not once per streamed piece. After the socket
 * connected again the events it missed are read.
 *
 * A chat is opened at its last message and read backwards from there, a page of events at a time, until the window
 * holds `MESSAGE_WINDOW` entries or the log runs out. Reading a long chat from its first event made the page wait for
 * messages nobody was going to scroll up to — a run of a few thousand events was a wall of text before its first line
 * appeared — so what is above the window waits for `loadOlder`, which is what scrolling to the top of a conversation
 * is for.
 */
import { useCallback, useEffect, useRef, useState } from "react";
import { EVENT_PAGE, olderEvents, runEvents } from "../api/calls/chat";
import type { RunEvent } from "../api/types/chat";
import { useConnection, useServerEvents } from "../live/LiveProvider";
import { Conversation, type Entry } from "./conversation";

/** How many entries the window holds before the read stops and leaves the rest of the chat above it. */
export const MESSAGE_WINDOW = 64;

/**
 * The most pages the first read fetches before it shows what it has. A chat whose events fold into few entries — a
 * very long answer streamed in thousands of pieces — would otherwise be read to its beginning before anything is
 * drawn as loaded; the rest waits for `loadOlder` like any other part above the window.
 */
export const MAX_FIRST_PAGES = 20;

export interface Thread {
  entries: Entry[];
  /** The highest `seq` the page holds; 0 when none. */
  lastSeq: number;
  /** Whether the first read of the chat finished. */
  loaded: boolean;
  /** Why the events could not be read, if they could not. The entries already read stay in `entries`. */
  error: string | null;
  /** Whether events of the chat are held above what is drawn, so there is more to read. */
  hasOlder: boolean;
  /** Whether the page above the ones drawn is being read. */
  olderLoading: boolean;
  /** Reads the events above what is drawn. Does nothing when there are none, or while one is on its way. */
  loadOlder: () => void;
}

/** What one chat's follow holds between renders. */
interface Reading {
  id: number;
  value: Conversation;
  loaded: boolean;
  /** Whether events are known to be above the oldest one held. */
  older: boolean;
  /** Whether the first read is still fetching pages. */
  reading: boolean;
  /** Whether the page above the ones held is being read. */
  olderLoading: boolean;
  /** Whether the stream missed events while the first read was still going, to be read when it ends. */
  missed: boolean;
  /** Why the latest read failed; kept for events of the stream to publish while nothing has been read. */
  error: string | null;
}

const EMPTY: Thread = {
  entries: [],
  lastSeq: 0,
  loaded: false,
  error: null,
  hasOlder: false,
  olderLoading: false,
  // A chat that is not open has nothing above its first message.
  loadOlder: () => {},
};

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
  const conversation = useRef<Reading | null>(null);
  const frame = useRef<number | null>(null);
  /** The number of the latest read; a result of an older one does not touch the page. */
  const latestRead = useRef(0);
  const connection = useConnection();

  const publish = (error: string | null) => {
    const current = conversation.current;
    if (!current) return;
    current.error = error;
    setThread({
      entries: current.value.entries(),
      lastSeq: current.value.lastSeq,
      loaded: current.loaded,
      error,
      hasOlder: current.older,
      olderLoading: current.olderLoading,
      loadOlder: () => loadOlder(current),
    });
  };

  /** Why something failed, in the words it came with. */
  const reason = (e: unknown) => (e instanceof Error ? e.message : "cannot read the chat");

  // Reads what the conversation does not hold yet, from the newest event it has upwards. Reads are ordinary requests
  // that need no socket, so a chat is read even while the stream is trying to connect again: a laptop waking from
  // sleep used to leave the page on "Reading the chat" until the socket opened, and never mark the chat as read at
  // all. What a read brings is always taken in (reading the same events twice is harmless), but only the latest read
  // says whether the page shows a failure, and a failure never takes a conversation that is on screen away: the page
  // shows it above the entries.
  function refill(current: Reading): void {
    if (current.reading) {
      // The first read has not reached the end of the chat yet; it reads what was missed along with the rest.
      current.missed = true;
      return;
    }
    if (!current.loaded) {
      // The first read failed, and the stream has brought events since: reading only after them would call the chat
      // read with its history out of reach, so the end of the chat is read again — below what the stream brought —
      // and what was missed is read once it has.
      current.missed = true;
      readEnd(current);
      return;
    }
    const read = ++latestRead.current;
    const after = current.value.lastSeq;
    runEvents(current.id, after)
      .then((events) => {
        if (conversation.current !== current) return;
        current.value.backfill(events);
        // Nothing was held, so the whole log was read and there is nothing above it to ask for.
        if (after === 0) current.older = false;
        current.loaded = true;
        if (read === latestRead.current) publish(null);
      })
      .catch((e: unknown) => {
        if (conversation.current !== current || read !== latestRead.current) return;
        publish(reason(e));
      });
  }

  // Reads the end of the chat: a page of events at a time, each one above the last, until the window has enough
  // entries to read or the log runs out. Each page is shown as it arrives, so a long chat fills in from its end
  // rather than appearing all at once after the last request.
  function readEnd(current: Reading): void {
    const read = ++latestRead.current;
    current.reading = true;
    let pages = 0;
    const page = () => {
      pages += 1;
      const below = current.value.oldestSeq;
      olderEvents(current.id, below === 0 ? null : below)
        .then((events) => {
          if (conversation.current !== current || read !== latestRead.current) return;
          current.value.backfill(events);
          // A page that came back full says there was more where it came from, which is all a person needs to be
          // told: the events themselves say how far back they go.
          current.older = events.length >= EVENT_PAGE;
          if (
            current.older &&
            pages < MAX_FIRST_PAGES &&
            current.value.entries().length < MESSAGE_WINDOW
          ) {
            publish(null);
            page();
            return;
          }
          current.reading = false;
          current.loaded = true;
          if (current.missed) {
            current.missed = false;
            refill(current);
            return;
          }
          publish(null);
        })
        .catch((e: unknown) => {
          if (conversation.current !== current || read !== latestRead.current) return;
          current.reading = false;
          current.olderLoading = false;
          publish(reason(e));
        });
    };
    page();
  }

  // Reads the events above the ones drawn: one page, which is what a person scrolling to the top of a conversation
  // asked for. It is read again from wherever this one stopped, so the whole chat is reachable one scroll at a time.
  function loadOlder(current: Reading): void {
    if (!current.older || current.reading || current.olderLoading) return;
    current.olderLoading = true;
    publish(null);
    olderEvents(current.id, current.value.oldestSeq)
      .then((events) => {
        if (conversation.current !== current) return;
        current.value.backfill(events);
        current.older = events.length >= EVENT_PAGE;
        current.olderLoading = false;
        publish(null);
      })
      .catch((e: unknown) => {
        if (conversation.current !== current) return;
        current.olderLoading = false;
        publish(reason(e));
      });
  }

  // biome-ignore lint/correctness/useExhaustiveDependencies: it only reads refs and the functions that do
  const loadOlderNow = useCallback(() => {
    const current = conversation.current;
    if (current) loadOlder(current);
  }, []);

  // Reads the chat when it is opened, and what it missed whenever the socket is (again) online.
  // biome-ignore lint/correctness/useExhaustiveDependencies: `publish`, `refill` and `readEnd` only read refs and set state
  useEffect(() => {
    if (id === null) {
      conversation.current = null;
      setThread(EMPTY);
      return;
    }
    if (conversation.current?.id !== id) {
      conversation.current = {
        id,
        value: new Conversation(id),
        loaded: false,
        older: false,
        reading: false,
        olderLoading: false,
        missed: false,
        error: null,
      };
      setThread({ ...EMPTY, loadOlder: loadOlderNow });
      readEnd(conversation.current);
      return;
    }
    refill(conversation.current);
  }, [id, connection]);

  useServerEvents((event) => {
    const current = conversation.current;
    if (!current) return;
    if (event.kind === "lagged") {
      // The server dropped this client's events rather than queue them, so the conversation has a hole in its `seq`s
      // that nothing else fills: the unread marker would never be satisfied and part of the answer would be simply
      // missing. What was missed is still stored, so it is read again.
      refill(current);
      return;
    }
    const stored = runEventOf(current.id, event);
    if (!stored) return;
    current.value.push(stored);
    if (frame.current !== null) return;
    frame.current = requestAnimationFrame(() => {
      frame.current = null;
      // A chat that could not be read yet keeps saying so while the stream brings events: they are not the history.
      const current = conversation.current;
      if (current) publish(current.loaded ? null : current.error);
    });
  });

  useEffect(
    () => () => {
      // The cancelled frame is no longer coming, so the flag has to go with it or nothing would ever schedule another.
      if (frame.current !== null) cancelAnimationFrame(frame.current);
      frame.current = null;
    },
    [],
  );

  return thread;
}
