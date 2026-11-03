/**
 * One chat, open: its conversation (virtualised, following the answer; or Pi-Bolt's terminal interface in its place), the change it offered, and the composer with
 * the pills of its place, model and reasoning; the agent it is written to is named by the field itself. The model and
 * the reasoning level change while the chat waits. Writing to a chat that ended goes on in a new chat that is told
 * this one. Once an answer is held its events are marked read, once per answer.
 */

import { PencilSimpleIcon } from "@phosphor-icons/react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import { Link, useNavigate, useSearchParams } from "react-router";
import {
  changeChatSettings,
  continueChat,
  markSeen,
  mergeChange,
  nameChat,
  offerChat,
  chats as readChats,
  say,
  stopRun,
} from "../api/calls/chat";
import { sayError } from "../api/errorText";
import { keys } from "../api/keys";
import type { Snapshot } from "../api/types/board";
import { type Chat, THINKING_LEVELS } from "../api/types/chat";
import { Terminal } from "../terminal/LazyTerminal";
import { type PillOption, PillSelect } from "../ui/chat/PillSelect";
import { Tag } from "../ui/chat/Tag";
import { Button } from "../ui/components/Button";
import { Tooltip } from "../ui/components/Tooltip";
import { cn } from "../ui/lib/cn";
import { CONTROL } from "../ui/lib/field";
import { plural } from "../ui/lib/format";
import { Empty } from "../ui/page/Empty";
import { FadedLine } from "../ui/page/FadedLine";
import { Notice } from "../ui/page/Notice";
import { Composer } from "./Composer";
import { draftKey, readDraft, saveDraft } from "./drafts";
import { endedHint, isOpen, isWorking, statusTag, takesMessage } from "./format";
import { offeredChange, placeName } from "./places";
import { seqToMark } from "./readMarkers";
import { Starters } from "./Starters";
import { ThreadView } from "./ThreadView";
import { useChoices } from "./useChoices";
import { useThread } from "./useThread";

/**
 * The title row of the top bar for a chat: its name, which the person may change, and where its status stands. Who
 * the chat is with is not said here: the pills under the field say it, and they are where that choice lives. The way
 * to name it is a pencil that comes out of the quiet only while the row is hovered or the field has focus, so the
 * row reads as a title until it is needed.
 */
export function ChatTitle({ chat, onNotice }: { chat: Chat; onNotice: (notice: string) => void }) {
  const queryClient = useQueryClient();
  const [naming, setNaming] = useState(false);
  const [draft, setDraft] = useState(chat.title);
  const tag = statusTag(chat.status, chat.asks);
  const name = chat.title || "New chat";

  const field = useRef<HTMLInputElement>(null);
  // The name is there to be changed, so the field opens focused with the old name selected.
  useEffect(() => {
    if (naming) field.current?.select();
  }, [naming]);

  function start() {
    setDraft(chat.title);
    setNaming(true);
  }

  /** Writes the name, unless nothing was changed; Enter and leaving the field both save, Escape leaves. */
  async function save() {
    if (!naming) return;
    setNaming(false);
    const title = draft.trim();
    if (title === chat.title) return;
    try {
      const named = await nameChat(chat.id, title);
      queryClient.setQueryData<Chat[]>(keys.chats, (chats) =>
        chats?.map((each) => (each.id === named.id ? named : each)),
      );
    } catch (error) {
      onNotice(sayError(error, "Could not name the chat. Try again."));
    }
  }

  return (
    <>
      {naming ? (
        <input
          ref={field}
          aria-label="Name of the chat"
          value={draft}
          maxLength={80}
          onChange={(event) => setDraft(event.target.value)}
          onBlur={() => void save()}
          onKeyDown={(event) => {
            if (event.key === "Enter") void save();
            if (event.key === "Escape") setNaming(false);
          }}
          className={cn(CONTROL, "min-w-0 flex-1")}
          data-testid="chat-title-field"
        />
      ) : (
        <span className="group flex min-w-0 flex-1 items-center gap-1" data-testid="chat-title">
          <FadedLine
            text={name}
            keep="head"
            title={name}
            className="text-base font-semibold text-text"
          />
          <button
            type="button"
            onClick={start}
            aria-label={`Name the chat${chat.title ? "" : " (it has none yet)"}`}
            className={cn(
              "inline-flex size-10 shrink-0 cursor-pointer items-center justify-center rounded-control",
              "text-secondary opacity-0 transition-opacity duration-(--motion-routine) ease-out",
              "hover:bg-hover hover:text-text focus-visible:opacity-100",
              "group-hover:opacity-100 motion-reduce:transition-none",
            )}
          >
            <PencilSimpleIcon className="size-4" aria-hidden="true" />
          </button>
        </span>
      )}
      {tag && <Tag tone={tag.tone}>{tag.label}</Tag>}
    </>
  );
}

/** The right of the top bar while the chat lasts: offering what its worktree changes, and stopping its agent. */
export function ChatActions({
  chat,
  onNotice,
}: {
  chat: Chat;
  onNotice: (notice: string) => void;
}) {
  const queryClient = useQueryClient();
  const [confirming, setConfirming] = useState(false);
  const [params, setParams] = useSearchParams();
  if (!isOpen(chat.status)) return null;
  const inTerminal = params.get("terminal") === "1";
  const toggleTerminal = () =>
    setParams(
      (current) => {
        const next = new URLSearchParams(current);
        if (inTerminal) next.delete("terminal");
        else next.set("terminal", "1");
        return next;
      },
      { replace: true },
    );
  const working = isWorking(chat.status);

  async function offer() {
    try {
      const change = await offerChat(chat.id);
      onNotice(`Offered as a change on ${change.branch}: it waits in the Inbox, ready to merge.`);
    } catch (e) {
      onNotice(e instanceof Error ? e.message : "Couldn't offer the change. Try again.");
    }
    await queryClient.invalidateQueries({ queryKey: keys.chats });
    await queryClient.invalidateQueries({ queryKey: keys.snapshot });
  }

  async function stop() {
    setConfirming(false);
    try {
      await stopRun(chat.id);
    } catch (e) {
      onNotice(e instanceof Error ? e.message : "Couldn't stop the chat. Try again.");
    }
  }

  if (confirming) {
    return (
      <div className="flex flex-wrap items-center gap-2">
        <span className="text-base text-text">Stop this chat?</span>
        <Button variant="secondary" onClick={() => setConfirming(false)}>
          Keep going
        </Button>
        <Button onClick={() => void stop()}>Stop chat</Button>
      </div>
    );
  }
  const label = `Offer changes (${plural(chat.changed_files, "file", "files")})`;
  return (
    <div className="flex flex-wrap items-center gap-2">
      {inTerminal ? (
        <Button variant="secondary" onClick={toggleTerminal}>
          Conversation
        </Button>
      ) : chat.status === "waiting" ? (
        <Button variant="secondary" onClick={toggleTerminal}>
          Terminal
        </Button>
      ) : (
        <Tooltip content="The terminal opens once the agent has answered">
          <span>
            <Button variant="secondary" disabled>
              Terminal
            </Button>
          </span>
        </Tooltip>
      )}
      {chat.changed_files > 0 &&
        (working ? (
          <Tooltip content="Offer once the agent has answered">
            <span>
              <Button variant="secondary" disabled>
                {label}
              </Button>
            </span>
          </Tooltip>
        ) : (
          <Button variant="secondary" className="tabular-nums" onClick={() => void offer()}>
            {label}
          </Button>
        ))}
      {working && (
        <Button variant="secondary" onClick={() => setConfirming(true)}>
          Stop
        </Button>
      )}
    </div>
  );
}

function OfferedChange({ chat, snapshot }: { chat: Chat; snapshot: Snapshot | undefined }) {
  const queryClient = useQueryClient();
  const [error, setError] = useState<string | null>(null);
  const change = offeredChange(snapshot, chat.id);
  if (!change) return null;
  const label =
    change.state === "merged" ? "Merged" : change.state === "closed" ? "Closed" : "Change offered";
  async function merge() {
    if (!change) return;
    try {
      await mergeChange(change.id);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Couldn't merge the change. Try again.");
    }
    await queryClient.invalidateQueries({ queryKey: keys.snapshot });
  }
  return (
    <div
      className="flex shrink-0 flex-wrap items-center gap-3 rounded-card bg-surface p-4 shadow-card"
      data-testid="offered-change"
    >
      <Tag
        tone={
          change.state === "open" ? "attention" : change.state === "merged" ? "complete" : "plain"
        }
      >
        {label}
      </Tag>
      <span className="min-w-0 flex-1 break-all font-mono text-sm text-secondary">
        {change.branch}
      </span>
      {error && <span className="break-words text-sm text-error-text">{error}</span>}
      {change.state === "open" && (
        <Button size="sm" onClick={() => void merge()}>
          Merge
        </Button>
      )}
    </div>
  );
}

export function OpenChat({
  id,
  snapshot,
  notice,
  onNotice,
}: {
  id: number;
  snapshot: Snapshot | undefined;
  notice: string | null;
  onNotice: (notice: string | null) => void;
}) {
  const navigate = useNavigate();
  const [params] = useSearchParams();
  const queryClient = useQueryClient();
  const chats = useQuery({ queryKey: keys.chats, queryFn: readChats });
  const chat = chats.data?.find((each) => each.id === id);
  const thread = useThread(id);
  const choices = useChoices();
  const [message, setMessage] = useState(() => readDraft(draftKey(id)));
  const [sent, setSent] = useState(0);
  const [sending, setSending] = useState(false);
  const marked = useRef(0);
  // What the composer holds right now, which the closure of a send cannot see: an answer takes seconds, and what is
  // typed in those seconds is the next question, not something to throw away with the one that was sent.
  const held = useRef(message);

  // The message is the draft of this chat: leaving for another page and coming back finds it where it was left,
  // and sending it leaves nothing behind.
  const key = draftKey(id);
  function write(value: string) {
    held.current = value;
    setMessage(value);
    saveDraft(key, value);
  }
  function clear(text: string) {
    if (held.current.trim() !== text) return;
    held.current = "";
    setMessage("");
    saveDraft(key, "");
  }

  // Marks the chat read once an answer is held, not at every streamed piece of it.
  // biome-ignore lint/correctness/useExhaustiveDependencies: it runs when the answer, the marker or the events move
  useEffect(() => {
    const seq = seqToMark(chat, thread.lastSeq);
    if (seq === null || seq <= marked.current) return;
    marked.current = seq;
    queryClient.setQueryData<Chat[]>(keys.chats, (held) =>
      held?.map((each) => (each.id === id ? { ...each, seen: Math.max(each.seen, seq) } : each)),
    );
    markSeen(id, seq).catch(() => {
      marked.current = 0;
    });
  }, [id, chat?.answered, chat?.seen, thread.lastSeq]);

  if (!chat) {
    return (
      <div className="p-4">
        {chats.isSuccess ? (
          <Empty
            title="There is no such chat"
            body="It may have been deleted. Pick another chat from the list at the top, or start a new one."
            action={
              <Button asChild size="sm" variant="secondary">
                <Link to="/chat">Start a new chat</Link>
              </Button>
            }
          />
        ) : (
          <p role="status" className="m-0 text-base text-secondary">
            Reading the chat
          </p>
        )}
      </div>
    );
  }

  const open = isOpen(chat.status);
  const working = isWorking(chat.status);
  // A chat whose agent went away still takes a message: that chat is taken up again, told what was said in it, so the
  // person goes on where they are rather than in a new chat.
  const writesHere = takesMessage(chat.status);

  // Pi-Bolt's own terminal interface takes the place of the conversation; the top bar's button brings it back.
  if (open && params.get("terminal") === "1") {
    return (
      <div className="flex min-h-0 flex-1 flex-col gap-3" data-testid="chat-terminal">
        <Terminal path={`/v1/chats/${id}/tui`} autoFocus className="min-h-0 flex-1" />
        <p className="shrink-0 text-sm leading-relaxed text-secondary">
          Pi-Bolt's terminal interface continues this chat. Closing it, or Conversation above, goes
          back.
        </p>
      </div>
    );
  }

  async function send() {
    const text = message.trim();
    if (text === "" || sending || !chat) return;
    setSending(true);
    onNotice(null);
    setSent((n) => n + 1);
    try {
      if (writesHere) {
        await say(chat.id, text);
        clear(text);
        // Taking a chat up again says something new in its tag and in the list of chats, so both are read again
        // instead of waiting for the events to say so.
        if (!open) await queryClient.invalidateQueries({ queryKey: keys.chats });
      } else {
        // A chat that ended for good goes on in a new chat that is told this one.
        const run = await continueChat(chat.id, text);
        clear(text);
        await queryClient.invalidateQueries({ queryKey: keys.chats });
        void navigate(`/chat/${run.id}`);
      }
    } catch (e) {
      onNotice(e instanceof Error ? e.message : "Couldn't send the message. Try again.");
    }
    setSending(false);
  }

  async function change(settings: { model?: string; thinking?: string }) {
    if (!chat) return;
    try {
      await changeChatSettings(chat.id, settings);
      onNotice(null);
    } catch (e) {
      onNotice(e instanceof Error ? e.message : "Couldn't change the chat. Try again.");
    }
    await queryClient.invalidateQueries({ queryKey: keys.chats });
  }

  const agent = choices.agents.find((each) => each.name === chat.agent);
  const model = chat.model ?? (agent?.model || null);
  const thinking = chat.thinking ?? agent?.thinking ?? null;
  const waiting = chat.status === "waiting";
  const place = placeName(snapshot, chat);
  const modelOptions: PillOption[] = [
    ...(model && !choices.models.includes(model) ? [{ value: model, label: model }] : []),
    ...choices.models.map((value) => ({ value, label: value })),
  ];
  const levelOptions: PillOption[] = THINKING_LEVELS.map((level) => ({
    value: level,
    label: level,
  }));
  // Nothing is said under the composer for a chat that has ended: its own tag in the header already says so, and
  // writing to it is the ordinary thing the composer does.
  const hint = notice
    ? notice
    : working
      ? "The agent is working. Your message goes when it finishes this turn."
      : null;

  return (
    <>
      <div className="min-h-0 flex-1" data-testid="conversation">
        {/* A conversation already read stays on screen: a failed read shows its reason above it, not instead of it. */}
        {thread.error ? <Notice tone="error">{thread.error}</Notice> : null}
        {thread.error && !thread.loaded ? null : (
          <ThreadView
            entries={thread.entries}
            working={working}
            sent={sent}
            empty={thread.loaded ? "Start with one of these, or write your own." : undefined}
            emptyTitle={thread.loaded ? "Nothing said yet" : "Reading the chat"}
            // The starters are for the moment before anything is written; once a message is on its way they are in
            // the way, and picking one would replace what was typed.
            emptyAction={
              open && thread.loaded && message.trim() === "" ? (
                <Starters onPick={write} />
              ) : undefined
            }
          />
        )}
      </div>
      <OfferedChange chat={chat} snapshot={snapshot} />
      <Composer
        value={message}
        onChange={write}
        onSend={() => void send()}
        canSend={message.trim() !== "" && !sending}
        // A chat that takes no message says on the button what pressing it really does: it starts a chat that carries
        // this one on. The tag in the title row is still the only place the state itself is said.
        sendLabel={writesHere ? "Send" : "Continue"}
        sendTitle={writesHere ? undefined : "Starts a new chat, told what was said in this one"}
        // The field names who the message goes to, which is what the agent pill used to say and is the one thing
        // worth reading there. A chat that takes no message has nobody to write to, so the field says why, and what
        // the button does about it.
        placeholder={writesHere ? `Write to ${chat.agent}` : endedHint(chat.status, chat.agent)}
        pills={
          <>
            <Tag>{place ?? "no repository"}</Tag>
            {open && (
              <>
                <PillSelect
                  label="Model"
                  valueOnly
                  value={model}
                  placeholder="Server default"
                  options={modelOptions}
                  onChange={(value) => void change({ model: value })}
                  disabled={!waiting}
                  searchable
                />
                <PillSelect
                  label="Reasoning"
                  valueOnly
                  value={thinking}
                  placeholder="Not set"
                  options={levelOptions}
                  onChange={(value) => void change({ thinking: value })}
                  disabled={!waiting}
                />
              </>
            )}
          </>
        }
      />
      {hint && (
        <p
          className={cn(
            "shrink-0 break-words text-sm leading-relaxed",
            notice ? "text-text" : "text-secondary",
          )}
          data-testid="chat-hint"
        >
          {hint}
        </p>
      )}
    </>
  );
}
