/**
 * A chat that is not started: who it is with, where it works, on which model and with which reasoning, and the first
 * message. It is nothing on the server until Send, and the person leaves it by opening another chat. The Moebius
 * agent works without a model being chosen: when it has none of its own the server falls back to the Coder's, or,
 * with neither, refuses the chat with a sentence that names both tables, which is shown above the field.
 */
import { useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useNavigate } from "react-router";
import { startChat } from "../api/calls/chat";
import { keys } from "../api/keys";
import type { Snapshot } from "../api/types/board";
import { type ChatImage, type NewChat as NewChatBody, THINKING_LEVELS } from "../api/types/chat";
import { type PillOption, PillSelect } from "../ui/chat/PillSelect";
import { Tag } from "../ui/chat/Tag";
import { Composer } from "./Composer";
import { type HeldPicture, holdPictures, imageBodies, takesImages } from "./pictures";
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
  const [message, setMessage] = useState("");
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

  async function send() {
    if (!body || sending) return;
    setSending(true);
    setError(null);
    try {
      const run = await startChat(body);
      await queryClient.invalidateQueries({ queryKey: keys.chats });
      void navigate(`/chat/${run.id}`);
    } catch (e) {
      setError(e instanceof Error ? e.message : "cannot start the chat");
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
  const modelOptions: PillOption[] = [
    {
      value: "",
      label: chosen?.model ? `Agent's model (${chosen.model})` : "Server default",
    },
    ...choices.models.map((id) => ({ value: id, label: id })),
  ];
  const levelOptions: PillOption[] = [
    { value: "", label: chosen?.thinking ? `Agent's level (${chosen.thinking})` : "Not set" },
    ...THINKING_LEVELS.map((level) => ({ value: level, label: level })),
  ];

  const note = needsRepository
    ? "A grilling needs a repository: choose one below."
    : looks === false
      ? "This model answers in words only, so the message cannot carry a picture. Choose one that looks at them, under Model."
      : repository === HARNESS
        ? 'The agent works on a branch of the harness configuration repository: agents, skills, extensions, MCP servers and rules. "Offer changes" offers what it built as a change.'
        : agent === GRILLING_AGENT
          ? "The Grilling partner questions your idea in a repository. Ending the chat puts the issues it proposes in the Inbox, to confirm."
          : 'With a repository the agent works on its own branch. "Offer changes" offers what it built as a change.';

  return (
    <div className="flex min-h-0 flex-1 flex-col justify-end gap-[14px]" data-testid="new-chat">
      <div className="rounded-xl border bg-panel px-[18px] py-double">
        <h1 className="text-[15px] font-bold text-high">New chat</h1>
        <p className="mt-half text-sm text-normal">{note}</p>
        <div className="mt-base flex flex-wrap gap-base">
          <Tag>
            {agent}
            {repository === "" ? ", no repository" : ""}
          </Tag>
          {chosen && chosen.model === "" && model === "" && (
            <Tag tone="amber">No model of its own: the server falls back to the Coder's</Tag>
          )}
        </div>
      </div>
      <Composer
        value={message}
        onChange={setMessage}
        onSend={() => void send()}
        canSend={body !== null && !sending}
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
