import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

type ModelDescriptor = { input?: readonly string[] } | undefined;
type ContentBlock = { type: string; [key: string]: unknown };
type ToolResultDescriptor = { toolName: string; details?: unknown };

const OMITTED_IMAGE_NOTE =
  "Image attachment omitted because the active Pi model does not support image input.";

export function modelSupportsImages(model: ModelDescriptor): boolean {
  return model?.input?.includes("image") === true;
}

export function omitUnsupportedImages(
  content: readonly ContentBlock[],
  model: ModelDescriptor,
): ContentBlock[] | undefined {
  if (modelSupportsImages(model) || !content.some((block) => block.type === "image")) {
    return undefined;
  }

  return [
    ...content.filter((block) => block.type !== "image"),
    { type: "text", text: OMITTED_IMAGE_NOTE },
  ];
}

export function isSkyCuaObserveResult(event: ToolResultDescriptor): boolean {
  const toolName = event.toolName.toLowerCase();
  if (toolName === "observe" || toolName.endsWith("_observe")) return true;
  if (toolName !== "mcp" || typeof event.details !== "object" || event.details === null) {
    return false;
  }
  const details = event.details as Record<string, unknown>;
  return details.server === "sky_cua" && details.tool === "observe";
}

export default function skyCuaImageCapability(pi: ExtensionAPI) {
  pi.on("tool_result", (event, ctx) => {
    if (!isSkyCuaObserveResult(event)) return undefined;
    const content = omitUnsupportedImages(event.content, ctx.model);
    return content === undefined ? undefined : { content };
  });
}
