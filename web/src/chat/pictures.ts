/**
 * The pictures a person attaches to a chat message. A chat takes them only on a model that looks at images, so a page
 * asks `takesImages` before it lets a message carry one; the limits here are the server's (`app::chats` in the core),
 * checked here so a person hears before sending rather than after.
 */
import type { AvailableModels, ChatImage } from "../api/types/chat";

/** The kinds of picture both harnesses take, and what a browser calls them. */
export const IMAGE_KINDS = ["image/png", "image/jpeg", "image/gif", "image/webp"];

/** The most pictures one message carries, and the most bytes each holds. */
export const MAX_IMAGES = 6;
export const MAX_IMAGE_BYTES = 5 * 1024 * 1024;

/** A picture the composer holds before it is sent: what it is called, which kind it is, its bytes in base64, and a
 * web address that shows them, which is how the composer draws it. */
export interface HeldPicture extends ChatImage {
  /** Stable across renders, so holding or dropping one picture does not move the others. */
  key: string;
  /** A `data:` address of the same bytes, which is what the composer shows. */
  url: string;
}

/** Which kind a file of these bytes and this name is, as the server works it out too: what the browser says, or the
 * name's extension when the browser says nothing. */
export function kindOf(file: File): string | null {
  const named = extensionOf(file.name);
  const said = file.type === "" ? null : file.type;
  const kind = said ?? kindOfExtension(named);
  return kind !== null && IMAGE_KINDS.includes(kind) ? kind : null;
}

function extensionOf(name: string): string {
  const dot = name.lastIndexOf(".");
  return dot < 0 ? "" : name.slice(dot + 1).toLowerCase();
}

function kindOfExtension(extension: string): string | null {
  if (extension === "jpg" || extension === "jpeg") return "image/jpeg";
  if (extension === "png") return "image/png";
  if (extension === "gif") return "image/gif";
  if (extension === "webp") return "image/webp";
  return null;
}

/** The pictures of `files` the composer can hold, and a sentence for each it must leave out. `already` is how many it
 * holds already, which is what is left of the six a message carries. */
export async function holdPictures(
  files: readonly File[],
  already: number,
): Promise<{ held: HeldPicture[]; refused: string[] }> {
  const held: HeldPicture[] = [];
  const refused: string[] = [];
  for (const file of files) {
    const kind = kindOf(file);
    const name = file.name === "" ? "the picture on the clipboard" : file.name;
    if (kind === null) {
      refused.push(`${name} is not a picture a chat takes`);
      continue;
    }
    if (file.size > MAX_IMAGE_BYTES) {
      refused.push(`${name} is larger than ${Math.floor(MAX_IMAGE_BYTES / (1024 * 1024))} MB`);
      continue;
    }
    if (already + held.length >= MAX_IMAGES) {
      refused.push(`a message carries ${MAX_IMAGES} pictures at most`);
      continue;
    }
    try {
      const data = await base64Of(file);
      held.push({
        key: `${name}-${file.size}-${held.length}`,
        name: file.name === "" ? undefined : file.name,
        media_type: kind,
        data,
        url: `data:${kind};base64,${data}`,
      });
    } catch {
      refused.push(`${name} could not be read`);
    }
  }
  return { held, refused };
}

/** The pictures a drop or a paste carries. Only files that are pictures count, so dropping text or a link goes on as
 * a plain drop would. */
export function pictureFiles(data: DataTransfer | null): File[] {
  if (data === null) return [];
  const files = Array.from(data.files ?? []);
  // A paste into some browsers lands only in `items`, with no name to go by.
  const items = Array.from(data.items ?? [])
    .filter((item) => item.kind === "file")
    .map((item) => item.getAsFile())
    .filter((file): file is File => file !== null);
  const seen = new Set<string>();
  return [...files, ...items].filter((file) => {
    if (kindOf(file) === null) return false;
    const id = `${file.name}:${file.size}:${file.type}`;
    if (seen.has(id)) return false;
    seen.add(id);
    return true;
  });
}

/** The bodies the server takes, without what the composer keeps to draw the picture. */
export function imageBodies(held: readonly HeldPicture[]): ChatImage[] {
  return held.map(({ key: _key, url: _url, ...image }) => image);
}

/** Whether the model a chat runs on looks at pictures: the one the person chose, or the agent's without one. A class
 * looks at them only when every model of it does, since a chat cannot know which of them answers; a model the server
 * does not list is left to the server, which asks the harness rather than refusing here — that is `null`. */
export function takesImages(
  available: Pick<AvailableModels, "models" | "classes"> | undefined,
  chosen: string | null | undefined,
  agentModel: string | null | undefined,
): boolean | null {
  const wanted = (chosen ?? "").trim() === "" ? (agentModel ?? "").trim() : (chosen ?? "").trim();
  if (available === undefined || wanted === "") return null;
  const model = available.models.find((each) => each.id === wanted);
  if (model !== undefined) return model.images;
  const named = available.classes.find((each) => each.name === wanted);
  return named !== undefined ? named.images : null;
}

async function base64Of(file: File): Promise<string> {
  const bytes = new Uint8Array(await file.arrayBuffer());
  let chunk = "";
  // `fromCharCode` over the whole file at once overflows the argument list of a big picture.
  for (let at = 0; at < bytes.length; at += 0x8000) {
    chunk += String.fromCharCode(...bytes.subarray(at, at + 0x8000));
  }
  return btoa(chunk);
}
