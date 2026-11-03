/**
 * One turn of a conversation (the desktop app's CH3-D): the person's message as the head of a card and what the agent
 * did and answered as its body. Presentational: the turn comes in as data (see `chat/turns.ts`), and so do the folded
 * state and the clicks. A later page that shows a run's conversation draws it with this.
 */
import { CircleNotchIcon } from "@phosphor-icons/react";
import { useState } from "react";
import { inputSummary, refused, type ToolCall, toolTitle, verdict } from "../../chat/conversation";
import type { Block, StepView, Turn as TurnData } from "../../chat/turns";
import { cn } from "../lib/cn";
import { ChatAggregatedToolEntries } from "./ChatAggregatedToolEntries";
import { ChatAssistantMessage } from "./ChatAssistantMessage";
import { ChatCollapsedThinking } from "./ChatCollapsedThinking";
import { ChatErrorMessage } from "./ChatErrorMessage";
import { ChatFileEntry } from "./ChatFileEntry";
import { ChatMarkdown } from "./ChatMarkdown";
import { ChatTodoList } from "./ChatTodoList";
import { ChatToolSummary } from "./ChatToolSummary";
import { ChatUserMessage } from "./ChatUserMessage";
import { CopyButton } from "./CopyButton";
import { DiffBody, type DiffChange, diffStats } from "./DiffBody";
import { Markdown } from "./Markdown";
import type { ToolStatusLike } from "./ToolStatusDot";

/** The most lines of a tool's output drawn; the rest is said to be left out. */
const MAX_OUTPUT_LINES = 200;

/** The status the dot of a tool call shows. */
export function toolStatus(call: ToolCall): ToolStatusLike {
  if (refused(call)) return { status: "denied" };
  if (call.execution?.state === "finished") {
    return { status: call.execution.failed ? "failed" : "success" };
  }
  return { status: "created" };
}

/** The changes a file entry shows: an edit's replacements, or a written file as all added. */
function changesOf(call: ToolCall): DiffChange[] | null {
  const input = call.input;
  if (input.type === "edit") return input.changes;
  if (input.type === "write") return [{ oldText: "", newText: input.content }];
  return null;
}

function Details({ call }: { call: ToolCall }) {
  const check = verdict(call);
  const output = call.output.split("\n");
  const shown = output.slice(0, MAX_OUTPUT_LINES).join("\n");
  const input =
    call.input.type === "command"
      ? call.input.command
      : call.input.type === "other"
        ? call.input.json
        : inputSummary(call.input);
  return (
    <div className="ml-6 mt-half flex flex-col gap-half rounded-md border bg-recessed p-base">
      {check && !check.allowed && (
        <div className="text-xs text-error">
          Blocked: {check.reason || "a rule refused this call"}
        </div>
      )}
      {input && (
        <div className="flex items-start gap-base">
          <pre className="min-w-0 flex-1 whitespace-pre-wrap break-all font-mono text-xs text-high">
            {input}
          </pre>
          <CopyButton text={input} label="Copy input" />
        </div>
      )}
      {call.output !== "" && (
        <div className="flex items-start gap-base border-t pt-half">
          <pre className="max-h-64 min-w-0 flex-1 overflow-auto whitespace-pre-wrap break-all font-mono text-xs text-normal">
            {shown}
            {output.length > MAX_OUTPUT_LINES &&
              `\n… ${output.length - MAX_OUTPUT_LINES} more lines left out`}
          </pre>
          <CopyButton text={call.output} label="Copy output" />
        </div>
      )}
    </div>
  );
}

function ToolLine({
  call,
  expanded,
  onToggle,
}: {
  call: ToolCall;
  expanded: boolean;
  onToggle: () => void;
}) {
  const status = toolStatus(call);
  const check = verdict(call);
  const changes = changesOf(call);
  if (call.input.type === "todos") {
    return <ChatTodoList todos={call.input.todos} expanded={expanded} onToggle={onToggle} />;
  }
  if (changes && (call.input.type === "edit" || call.input.type === "write") && !refused(call)) {
    const { additions, deletions } = diffStats(changes);
    return (
      <ChatFileEntry
        filename={call.input.path}
        additions={additions}
        deletions={deletions}
        status={status}
        expanded={expanded}
        onToggle={onToggle}
      >
        <DiffBody changes={changes} />
      </ChatFileEntry>
    );
  }
  const tag = refused(call) ? (
    <span className="shrink-0 rounded-sm bg-error/10 px-base text-xs text-error">Blocked</span>
  ) : call.execution?.state === "running" ? (
    <span className="shrink-0 rounded-sm bg-live-fill px-base text-xs text-live">Running</span>
  ) : null;
  const hasDetails =
    call.output !== "" || (check !== null && !check.allowed) || call.input.type !== "unknown";
  return (
    <div>
      <ChatToolSummary
        summary={inputSummary(call.input)}
        toolName={toolTitle(call)}
        status={status}
        trailing={tag}
        isTruncated={hasDetails}
        expanded={expanded}
        onToggle={onToggle}
        className={cn(refused(call) && "text-error")}
      />
      {expanded && <Details call={call} />}
    </div>
  );
}

function StepLine({
  step,
  isOpen,
  onToggle,
}: {
  step: StepView;
  isOpen: (key: string) => boolean;
  onToggle: (key: string) => void;
}) {
  const [hovered, setHovered] = useState(false);
  if (step.kind === "thinking") {
    return (
      <ChatCollapsedThinking
        entries={[{ content: step.text, expansionKey: step.key }]}
        expanded={isOpen(step.key)}
        isHovered={hovered}
        onToggle={() => onToggle(step.key)}
        onHoverChange={setHovered}
        renderMarkdown={({ content, className }) => (
          <Markdown content={content} className={className} />
        )}
      />
    );
  }
  if (step.kind === "reads") {
    return (
      <ChatAggregatedToolEntries
        label="Read"
        unit="file"
        entries={step.calls.map((call, index) => ({
          summary: inputSummary(call.input),
          status: toolStatus(call),
          expansionKey: `${step.key}-${index}`,
        }))}
        expanded={isOpen(step.key)}
        isHovered={hovered}
        onToggle={() => onToggle(step.key)}
        onHoverChange={setHovered}
      />
    );
  }
  return (
    <ToolLine call={step.call} expanded={isOpen(step.key)} onToggle={() => onToggle(step.key)} />
  );
}

function BlockView({
  block,
  isOpen,
  onToggle,
}: {
  block: Block;
  isOpen: (key: string) => boolean;
  onToggle: (key: string) => void;
}) {
  switch (block.kind) {
    case "text":
      return (
        <ChatAssistantMessage
          content={block.text}
          renderMarkdown={({ content }) => (
            <ChatMarkdown
              content={content}
              renderContent={({ content, className }) => (
                <Markdown content={content} className={className} />
              )}
            />
          )}
          actions={<CopyButton text={block.text} label="Copy message" />}
        />
      );
    case "error":
      return (
        <ChatErrorMessage
          title={block.source === "model" ? "The model returned an error" : "The run failed"}
          content={block.message}
          actions={<CopyButton text={block.message} label="Copy error" />}
        />
      );
    case "mcpError":
      return (
        <ChatErrorMessage
          title={`MCP server ${block.server} could not be reached`}
          content={block.message}
          actions={<CopyButton text={block.message} label="Copy error" />}
        />
      );
    case "steps":
      return (
        <div className="flex flex-col gap-base" data-testid="steps">
          {block.steps.map((step) => (
            <StepLine key={step.key} step={step} isOpen={isOpen} onToggle={onToggle} />
          ))}
          {block.working && (
            <div className="flex items-center gap-base text-sm text-low" data-testid="working">
              <CircleNotchIcon className="size-icon-base animate-spin" />
              Working…
            </div>
          )}
        </div>
      );
  }
}

interface TurnProps {
  turn: TurnData;
  /** The keys of the steps that are unfolded. */
  unfolded: ReadonlySet<string>;
  onToggle: (key: string) => void;
}

export function Turn({ turn, unfolded, onToggle }: TurnProps) {
  const isOpen = (key: string) => unfolded.has(key);
  const head = turn.head;
  return (
    <section data-testid="turn" className="overflow-hidden rounded-xl border bg-panel shadow-md">
      {head && (
        <ChatUserMessage
          content={head.message}
          title={head.said === "steered" ? "You steered the chat" : "You"}
          className={cn("rounded-none border-0 bg-card", turn.blocks.length > 0 && "border-b")}
          headerRight={<CopyButton text={head.message} label="Copy your message" />}
          renderMarkdown={({ content }) => (
            <>
              <p
                className={cn(
                  "whitespace-pre-wrap break-words px-double text-sm font-semibold text-high",
                  head.pictures.length === 0 && "pb-plusfifty",
                )}
                data-testid="person-message"
              >
                {content}
              </p>
              {/* The pictures the message was written with, in a row under it; each opens on its own. */}
              {head.pictures.length > 0 && (
                <ul
                  aria-label="Pictures with the message"
                  className="flex flex-wrap gap-half px-double pb-plusfifty"
                >
                  {head.pictures.map((picture) => (
                    <li key={picture.key}>
                      <a
                        href={picture.src}
                        target="_blank"
                        rel="noreferrer"
                        title={`${picture.name ?? "A picture"} (${picture.mediaType})`}
                      >
                        {/* The bytes are served as they were sent, so the picture is seen as it was pasted. */}
                        <img
                          src={picture.src}
                          alt={picture.name ?? "A picture sent with the message"}
                          className="max-h-40 rounded-md border border-line object-contain"
                        />
                      </a>
                    </li>
                  ))}
                </ul>
              )}
            </>
          )}
        />
      )}
      {turn.blocks.length > 0 && (
        <div className="flex flex-col gap-plusfifty px-double py-double">
          {turn.blocks.map((block) => (
            <BlockView key={block.key} block={block} isOpen={isOpen} onToggle={onToggle} />
          ))}
        </div>
      )}
    </section>
  );
}
