/**
 * One chat, open: its conversation (virtualised, following the answer; or Pi-Bolt's terminal interface in its place), the change it offered, and the composer with
 * the pills of its agent, place, model and reasoning. The model and the reasoning level change while the chat waits.
 * Writing to a chat that ended goes on in a new chat that is told this one. Once an answer is held its events are
 * marked read, once per answer.
 */
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import { useNavigate, useSearchParams } from "react-router";
import {
  changeChatSettings,
  continueChat,
  markSeen,
  mergeChange,
  offerChat,
  chats as readChats,
  say,
  stopRun,
} from "../api/calls/chat";
import { keys } from "../api/keys";
import type { Snapshot } from "../api/types/board";
import { type Chat, THINKING_LEVELS } from "../api/types/chat";
import { Terminal } from "../terminal/Terminal";
import { type PillOption, PillSelect } from "../ui/chat/PillSelect";
import { Tag } from "../ui/chat/Tag";
import { Button } from "../ui/components/Button";
import { Tooltip } from "../ui/components/Tooltip";
import { Composer } from "./Composer";
import { isOpen, isWorking, statusTag } from "./format";
import { type HeldPicture, holdPictures, imageBodies, takesImages } from "./pictures";
import { offeredChange, placeName } from "./places";
import { seqToMark } from "./readMarkers";
import { ThreadView } from "./ThreadView";
import { useChoices } from "./useChoices";
import { useThread } from "./useThread";

/** The title row of the top bar for a chat: its title, who it is with, and where its status stands. */
export function ChatTitle({ chat, snapshot }: { chat: Chat; snapshot: Snapshot | undefined }) {
  const place = placeName(snapshot, chat);
  const tag = statusTag(chat.status, chat.asks);
  return (
    <>
      <span
        className="min-w-0 flex-1 truncate text-sm font-bold text-high"
        data-testid="chat-title"
      >
        {chat.title || "New chat"}
      </span>
      <Tag>{place ? `${chat.agent} · ${place}` : `${chat.agent} · no repository`}</Tag>
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
      onNotice(e instanceof Error ? e.message : "cannot offer the change");
    }
    await queryClient.invalidateQueries({ queryKey: keys.chats });
    await queryClient.invalidateQueries({ queryKey: keys.snapshot });
  }

  async function stop() {
    setConfirming(false);
    try {
      await stopRun(chat.id);
    } catch (e) {
      onNotice(e instanceof Error ? e.message : "cannot stop the chat");
    }
  }

  if (confirming) {
    return (
      <div className="flex items-center gap-base">
        <span className="font-mono text-xs text-normal">Stop this chat?</span>
        <Button variant="outline" size="xs" onClick={() => setConfirming(false)}>
          Keep going
        </Button>
        <Button variant="outline" size="xs" onClick={() => void stop()}>
          Stop chat
        </Button>
      </div>
    );
  }
  const label = `Offer changes (${chat.changed_files} ${chat.changed_files === 1 ? "file" : "files"})`;
  return (
    <div className="flex items-center gap-base">
      {inTerminal ? (
        <Button variant="outline" size="xs" onClick={toggleTerminal}>
          Conversation
        </Button>
      ) : chat.status === "waiting" ? (
        <Button variant="outline" size="xs" onClick={toggleTerminal}>
          Terminal
        </Button>
      ) : (
        <Tooltip content="The terminal opens once the agent has answered">
          <span>
            <Button variant="outline" size="xs" disabled>
              Terminal
            </Button>
          </span>
        </Tooltip>
      )}
      {chat.changed_files > 0 &&
        (working ? (
          <Tooltip content="Offer once the agent has answered">
            <span>
              <Button variant="outline" size="xs" disabled>
                {label}
              </Button>
            </span>
          </Tooltip>
        ) : (
          <Button variant="outline" size="xs" onClick={() => void offer()}>
            {label}
          </Button>
        ))}
      {working && (
        <Button variant="outline" size="xs" onClick={() => setConfirming(true)}>
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
      setError(e instanceof Error ? e.message : "cannot merge the change");
    }
    await queryClient.invalidateQueries({ queryKey: keys.snapshot });
  }
  return (
    <div
      className="flex shrink-0 items-center gap-base rounded-md border border-track bg-lane px-plusfifty py-base"
      data-testid="offered-change"
    >
      <Tag tone={change.state === "open" ? "amber" : change.state === "merged" ? "teal" : "plain"}>
        {label}
      </Tag>
      <span className="min-w-0 flex-1 truncate font-mono text-xs text-high">{change.branch}</span>
      {error && <span className="text-xs text-error">{error}</span>}
      {change.state === "open" && (
        <Button size="xs" onClick={() => void merge()}>
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
  const [message, setMessage] = useState("");
  const [pictures, setPictures] = useState<HeldPicture[]>([]);
  const [sent, setSent] = useState(0);
  const [sending, setSending] = useState(false);
  const marked = useRef(0);

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
      <p className="py-double text-center text-sm text-low">
        {chats.isSuccess ? "There is no such chat." : "Reading the chat…"}
      </p>
    );
  }

  const open = isOpen(chat.status);
  const working = isWorking(chat.status);

  // Pi-Bolt's own terminal interface takes the place of the conversation; the top bar's button brings it back.
  if (open && params.get("terminal") === "1") {
    return (
      <div className="flex min-h-0 flex-1 flex-col gap-half" data-testid="chat-terminal">
        <Terminal path={`/v1/chats/${id}/tui`} autoFocus className="min-h-0 flex-1" />
        <p className="shrink-0 font-mono text-xs text-low">
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
    const held = imageBodies(pictures);
    try {
      if (open) {
        await say(chat.id, text, undefined, held);
        setMessage("");
        setPictures([]);
      } else {
        // Writing to an ended chat goes on in a new chat that is told this one.
        const run = await continueChat(chat.id, text, held);
        setMessage("");
        setPictures([]);
        await queryClient.invalidateQueries({ queryKey: keys.chats });
        void navigate(`/chat/${run.id}`);
      }
    } catch (e) {
      onNotice(e instanceof Error ? e.message : "cannot send the message");
    }
    setSending(false);
  }

  /** Holds the pictures the person pasted, dropped or picked, and says what it had to leave out. */
  async function hold(files: File[]) {
    const taken = await holdPictures(files, pictures.length);
    if (taken.held.length > 0) setPictures((held) => [...held, ...taken.held]);
    if (taken.refused.length > 0) onNotice(taken.refused[0] ?? null);
  }

  async function change(settings: { model?: string; thinking?: string }) {
    if (!chat) return;
    try {
      await changeChatSettings(chat.id, settings);
      onNotice(null);
    } catch (e) {
      onNotice(e instanceof Error ? e.message : "cannot change the chat");
    }
    await queryClient.invalidateQueries({ queryKey: keys.chats });
  }

  const agent = choices.agents.find((each) => each.name === chat.agent);
  const model = chat.model ?? (agent?.model || null);
  // A chat takes pictures only on a model that looks at them; one whose model the server does not list is left to
  // the server, which asks the harness rather than refusing here.
  const looks = takesImages(choices.available, chat.model, agent?.model ?? null);
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
  const hint = notice
    ? notice
    : looks === false
      ? "This chat's model answers in words only, so it cannot look at a picture. Choose one that does, under Model."
      : working
        ? "The agent is working. Your message is queued and sent when it is done with this turn."
        : open
          ? "The agent answers at once."
          : "This chat has ended. What you send starts a new chat with the same agent, which is told this one.";

  return (
    <>
      <div className="min-h-0 flex-1" data-testid="conversation">
        {thread.error ? (
          <p role="alert" className="py-double text-center text-sm text-error">
            {thread.error}
          </p>
        ) : (
          <ThreadView
            entries={thread.entries}
            working={working}
            sent={sent}
            empty={thread.loaded ? "Nothing has been said in this chat yet." : "Reading the chat…"}
          />
        )}
      </div>
      <OfferedChange chat={chat} snapshot={snapshot} />
      <Composer
        value={message}
        onChange={setMessage}
        onSend={() => void send()}
        canSend={message.trim() !== "" && !sending}
        running={working}
        pictures={looks === false ? [] : pictures}
        onHold={looks === false ? undefined : (files) => void hold(files)}
        onUnhold={
          looks === false
            ? undefined
            : (picture) => setPictures((held) => held.filter((each) => each.key !== picture.key))
        }
        pills={
          <>
            <Tag>{chat.agent}</Tag>
            <Tag>{place ?? "no repository"}</Tag>
            {open && (
              <>
                <PillSelect
                  label="Model"
                  value={model}
                  placeholder="Server default"
                  options={modelOptions}
                  onChange={(value) => void change({ model: value })}
                  disabled={!waiting}
                  searchable
                />
                <PillSelect
                  label="Reasoning"
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
      <p className="shrink-0 font-mono text-xs text-low" data-testid="chat-hint">
        {hint}
      </p>
    </>
  );
}
