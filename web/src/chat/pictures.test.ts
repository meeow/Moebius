/** What a person hands the composer becomes what the server takes: a picture is kept only when it is one of the four
 * kinds a chat carries, small enough to send, and there is room for it in the message. */
import { describe, expect, it } from "vitest";
import {
  holdPictures,
  IMAGE_KINDS,
  imageBodies,
  kindOf,
  MAX_IMAGE_BYTES,
  MAX_IMAGES,
  pictureFiles,
  takesImages,
} from "./pictures";

/** The first bytes of a PNG, which is what the browser keeps when it hands a picture over. */
const PNG = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x01, 0x02, 0x03];
const PNG_BASE64 = btoa(String.fromCharCode(...PNG));

function picture(name: string, type: string, bytes: number[] = PNG): File {
  return new File([new Uint8Array(bytes)], name, { type });
}

describe("kindOf", () => {
  it("believes the browser, and falls back to the name when it says nothing", () => {
    expect(kindOf(picture("screenshot.png", "image/png"))).toBe("image/png");
    // The clipboard gives a picture no name, so only the extension can say what it is.
    expect(kindOf(picture("pasted image", ""))).toBeNull();
    expect(kindOf(picture("pasted-image.png", ""))).toBe("image/png");
    expect(kindOf(picture("holiday.JPG", ""))).toBe("image/jpeg");
    expect(kindOf(picture("photo.webp", ""))).toBe("image/webp");
  });

  it("takes only the kinds both harnesses read", () => {
    expect(kindOf(picture("diagram.bmp", "image/bmp"))).toBeNull();
    expect(kindOf(picture("diagram.svg", "image/svg+xml"))).toBeNull();
    expect(kindOf(picture("notes.txt", "text/plain"))).toBeNull();
    // What the browser calls it wins, so a JPEG kept under a PNG name is sent as a JPEG.
    expect(kindOf(picture("mislabeled.png", "image/jpeg"))).toBe("image/jpeg");
    expect(IMAGE_KINDS).toEqual(["image/png", "image/jpeg", "image/gif", "image/webp"]);
  });
});

describe("holdPictures", () => {
  it("keeps a picture with the name, the kind and the bytes to send", async () => {
    const taken = await holdPictures([picture("screenshot.png", "image/png")], 0);
    expect(taken.refused).toEqual([]);
    expect(taken.held).toHaveLength(1);
    const [held] = taken.held;
    expect(held?.name).toBe("screenshot.png");
    expect(held?.media_type).toBe("image/png");
    // The composer draws the picture from the same bytes the server is sent.
    expect(held?.url).toBe(`data:image/png;base64,${PNG_BASE64}`);
    expect(imageBodies(taken.held)).toEqual([
      { name: "screenshot.png", media_type: "image/png", data: PNG_BASE64 },
    ]);
  });

  it("holds a picture the clipboard gives without a name", async () => {
    const taken = await holdPictures([picture("", "image/png")], 0);
    expect(taken.refused).toEqual([]);
    const [held] = taken.held;
    expect(held?.name).toBeUndefined();
    expect(held?.media_type).toBe("image/png");
    // The server is told which kind it is even though the person never named it.
    expect(imageBodies(taken.held)).toEqual([
      { name: undefined, media_type: "image/png", data: PNG_BASE64 },
    ]);
  });

  it("leaves out a file that is not a picture a chat takes", async () => {
    const taken = await holdPictures([picture("notes.txt", "text/plain")], 0);
    expect(taken.held).toEqual([]);
    expect(taken.refused[0]).toContain("notes.txt");
    expect(taken.refused[0]).toContain("not a picture");
  });

  it("leaves out a picture bigger than the server reads", async () => {
    const bytes = new Array<number>(MAX_IMAGE_BYTES + 1).fill(0x89);
    const taken = await holdPictures([picture("huge.png", "image/png", bytes)], 0);
    expect(taken.held).toEqual([]);
    expect(taken.refused[0]).toContain("huge.png");
    expect(taken.refused[0]).toContain("5 MB");
  });

  it("fills the message and says the rest can go with the next one", async () => {
    const held = [];
    const refused = [];
    // One picture at a time, the way a person pastes them.
    for (let i = 0; i < MAX_IMAGES + 1; i++) {
      const taken = await holdPictures([picture(`p${i}.png`, "image/png")], held.length);
      held.push(...taken.held);
      refused.push(...taken.refused);
    }
    expect(held).toHaveLength(MAX_IMAGES);
    expect(refused).toEqual([`a message carries ${MAX_IMAGES} pictures at most`]);
  });

  it("counts the pictures already held, not the ones of this paste only", async () => {
    const taken = await holdPictures(
      [picture("a.png", "image/png"), picture("b.png", "image/png")],
      MAX_IMAGES - 1,
    );
    expect(taken.held.map((each) => each.name)).toEqual(["a.png"]);
    expect(taken.refused).toHaveLength(1);
  });
});

describe("pictureFiles", () => {
  it("takes the pictures a paste carries", () => {
    const file = picture("pasted.png", "image/png");
    const clipboard = { items: [{ kind: "file", getAsFile: () => file }], files: [], types: [] };
    expect(pictureFiles(clipboard as unknown as DataTransfer)).toEqual([file]);
  });

  it("takes the files a drop carries once, not once per list", () => {
    const file = picture("dropped.png", "image/png");
    const dropped = {
      items: [{ kind: "file", getAsFile: () => file }],
      files: [file],
      types: ["Files"],
    };
    expect(pictureFiles(dropped as unknown as DataTransfer)).toEqual([file]);
  });

  it("leaves text and non-pictures alone, which is what the field is for", () => {
    const typed = { items: [], files: [], types: ["text/plain"] };
    expect(pictureFiles(typed as unknown as DataTransfer)).toEqual([]);
    const mixed = {
      items: [],
      files: [picture("photo.png", "image/png"), picture("sheet.csv", "text/csv")],
      types: ["Files"],
    };
    expect(pictureFiles(mixed as unknown as DataTransfer).map((file) => file.name)).toEqual([
      "photo.png",
    ]);
    expect(pictureFiles(null)).toEqual([]);
  });
});

describe("takesImages", () => {
  const available = {
    models: [
      { id: "gpt-5", protocol: "openai", upstream: "u", images: true },
      { id: "gpt-5-nano", protocol: "openai", upstream: "u", images: false },
    ],
    errors: [],
    classes: [{ name: "strong", models: ["gpt-5"], protocols: ["openai"], images: true }],
  };

  it("asks the model the person chose", () => {
    expect(takesImages(available, "gpt-5", null)).toBe(true);
    expect(takesImages(available, "gpt-5-nano", "gpt-5")).toBe(false);
  });

  it("asks a class the chat runs on", () => {
    expect(takesImages(available, "strong", null)).toBe(true);
  });

  it("falls back to the model of the agent the chat runs on", () => {
    expect(takesImages(available, "", "gpt-5-nano")).toBe(false);
    expect(takesImages(available, null, "gpt-5")).toBe(true);
  });

  it("leaves a model the server does not list to the server", () => {
    expect(takesImages(available, "something-new", null)).toBeNull();
    expect(takesImages(undefined, "gpt-5", null)).toBeNull();
    expect(takesImages(available, "", null)).toBeNull();
  });
});
