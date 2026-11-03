/**
 * A chat that is not started: who it is with, where it works, on which model and with which reasoning, and the first
 * message. It is nothing on the server until Send, and the person leaves it by opening another chat. The Moebius
 * agent works without a model being chosen: when it has none of its own the server falls back to the Coder's, or,
 * with neither, refuses the chat with a sentence that names both tables, which is shown above the field. What has
 * been typed is kept by `drafts.ts`, so leaving for another page and coming back finds the message where it was left.
 */
import { useQueryClient } from "@tanstack/react-query";
import { type ReactNode, useState } from "react";
import { useNavigate } from "react-router";
import { startChat } from "../api/calls/chat";
import { keys } from "../api/keys";
import type { Snapshot } from "../api/types/board";
import type { ChatImage, NewChat as NewChatBody } from "../api/types/chat";
import { type PillOption, PillSelect } from "../ui/chat/PillSelect";
import { Tag } from "../ui/chat/Tag";
import { Empty } from "../ui/page/Empty";
import { Composer } from "./Composer";
import { draftKey, readDraft, saveDraft } from "./drafts";
import { levelChoices } from "./levels";
import { type HeldPicture, holdPictures, imageBodies, takesImages } from "./pictures";
import { Starters } from "./Starters";
import { useChoices } from "./useChoices";

export const DEFAULT_AGENT = "Moebius";

const GRILLING_AGENT = "Grilling-partner";
export const HARNESS = "harness";

/** What the chat to start is made of; `null` while it cannot start. A first message carries `pictures` only for a
 * model that looks at images, which is what the composer checked before offering them. */
export function newChatBody(draft: {
  agent: string;
  repository: string;
  model: string;
  thinking: string;
  message: string;
  pictures?: ChatImage[];
}): NewChatBody | null {
  const message = draft.message.trim();
  if (message === "") return null;
  if (draft.agent === GRILLING_AGENT && (draft.repository === "" || draft.repository === HARNESS)) {
    return null;
  }
  const body: NewChatBody = { agent: draft.agent, message };
  if (draft.pictures !== undefined && draft.pictures.length > 0) body.images = draft.pictures;
  if (draft.repository === HARNESS) body.harness = true;
  else if (draft.repository !== "") body.repository = Number(draft.repository);
  if (draft.model !== "") body.model = draft.model;
  if (draft.thinking !== "") body.thinking = draft.thinking;
  return body;
}

export function NewChat({ snapshot }: { snapshot: Snapshot | undefined }) {
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const choices = useChoices();
  const [agent, setAgent] = useState(DEFAULT_AGENT);
  const [repository, setRepository] = useState("");
  const [model, setModel] = useState("");
  const [thinking, setThinking] = useState("");
  const [message, setMessage] = useState(() => readDraft(draftKey(null)));
  const [pictures, setPictures] = useState<HeldPicture[]>([]);
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const chosen = choices.agents.find((each) => each.name === agent);
  // The chat looks at pictures when the chosen model does, or the agent's model without a choice; a model the server
  // does not list is left to the server, which asks the harness rather than refusing here.
  const looks = takesImages(choices.available, model, chosen?.model ?? null);
  const body = newChatBody({
    agent,
    repository,
    model,
    thinking,
    message,
    pictures: looks === false ? [] : imageBodies(pictures),
  });
  const needsRepository = agent === GRILLING_AGENT && body === null && message.trim() !== "";
  const noModel = chosen !== undefined && chosen.model === "" && model === "";

  // The message is kept as the draft of a chat not started yet, and dropped once it is sent.
  function write(value: string) {
    setMessage(value);
    saveDraft(draftKey(null), value);
  }

  async function send() {
    if (!body || sending) return;
    setSending(true);
    setError(null);
    try {
      const run = await startChat(body);
      await queryClient.invalidateQueries({ queryKey: keys.chats });
      saveDraft(draftKey(null), "");
      void navigate(`/chat/${run.id}`);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Couldn't start the chat. Try again.");
      setSending(false);
    }
  }

  /** Holds the pictures the person pasted, dropped or picked, and says what it had to leave out. */
  async function hold(files: File[]) {
    const taken = await holdPictures(files, pictures.length);
    if (taken.held.length > 0) setPictures((held) => [...held, ...taken.held]);
    if (taken.refused.length > 0) setError(taken.refused[0] ?? null);
  }

  const agentOptions: PillOption[] = (
    choices.agents.length > 0 ? choices.agents : [{ name: DEFAULT_AGENT, model: "" }]
  ).map((each) => ({ value: each.name, label: each.name, hint: each.model || undefined }));
  const placeOptions: PillOption[] = [
    { value: "", label: "No repository" },
    ...(snapshot?.boards.map((board) => ({
      value: String(board.repository.id),
      label: board.repository.name,
    })) ?? []),
    ...(choices.hasHarness ? [{ value: HARNESS, label: "Harness configuration" }] : []),
  ];
  // What the agent brings is shown as it is — its model's name, its level — and faded, because nothing has been
  // chosen yet. Picking a row underneath is what turns it into a choice, and into normal text.
  const modelOptions: PillOption[] = [
    { value: "", label: chosen?.model || "Server default", inherited: true },
    ...choices.models.map((id) => ({ value: id, label: id })),
  ];
  // Only the levels the model's endpoint takes; the agent's own is what the first row stands for.
  const levelOptions: PillOption[] = [
    { value: "", label: chosen?.thinking || "Not set", inherited: true },
    ...levelChoices(choices.available, model || chosen?.model || "", thinking),
  ];

  // Only what changes what the person is about to do: where the agent will work, and what it does with what it builds.
  const note = needsRepository
    ? "A grilling works in a repository. Choose one."
    : looks === false
      ? "This model answers in words only, so the message cannot carry a picture. Choose one that looks at them, under Model."
      : repository === HARNESS
        ? "The agent works on a branch of the harness configuration — its agents, skills and rules. Offer changes when it is done."
        : agent === GRILLING_AGENT
          ? "The Grilling partner questions the idea, then puts the issues it proposes in your Inbox to confirm."
          : null;

  // The starters are for the moment before anything is written; once a message is on its way they are in the way.
  const starters: ReactNode =
    agent === DEFAULT_AGENT && message.trim() === "" ? (
      <Empty
        title="Nothing said yet"
        body="Start with one of these, or write your own."
        className="mt-auto"
        action={<Starters onPick={write} />}
      />
    ) : null;

  return (
    <div className="flex min-h-0 flex-1 flex-col justify-end gap-4" data-testid="new-chat">
      {starters}
      {(note !== null || noModel) && (
        <div className="flex flex-col gap-2" data-testid="new-chat-note">
          {note !== null && (
            <p className="m-0 max-w-prose text-base leading-relaxed text-secondary">{note}</p>
          )}
          {noModel && (
            <p className="m-0 max-w-prose text-base leading-relaxed text-secondary">
              <Tag tone="attention" className="me-2 align-middle">
                No model
              </Tag>
              Moebius has none of its own: the Coder's is used unless you pick one.
            </p>
          )}
        </div>
      )}
      <Composer
        value={message}
        onChange={write}
        onSend={() => void send()}
        canSend={body !== null && !sending}
        sendHint={needsRepository ? "Choose a repository first" : undefined}
        error={error}
        pictures={looks === false ? [] : pictures}
        onHold={looks === false ? undefined : (files) => void hold(files)}
        onUnhold={
          looks === false
            ? undefined
            : (picture) => setPictures((held) => held.filter((each) => each.key !== picture.key))
        }
        autoFocus
        pills={
          <>
            <PillSelect
              label="Agent"
              value={agent}
              placeholder="Agent"
              options={agentOptions}
              onChange={setAgent}
            />
            <PillSelect
              label="Repository"
              value={repository}
              placeholder="No repository"
              options={placeOptions}
              onChange={setRepository}
            />
            <PillSelect
              label="Model"
              value={model}
              placeholder="Model"
              options={modelOptions}
              onChange={setModel}
              searchable
            />
            <PillSelect
              label="Reasoning"
              value={thinking}
              placeholder="Reasoning"
              options={levelOptions}
              onChange={setThinking}
            />
          </>
        }
      />
    </div>
  );
}
