import manifest from "./api-manifest.json";

type MemberDeclaration = {
  text: string;
  documented?: boolean;
  unsupportedByDefaultIn?: string[];
};

type MemberManifest = {
  declarations: MemberDeclaration[];
  documented?: boolean;
  unsupportedByDefaultIn?: string[];
};

type ApiManifest = {
  root: string;
  interfaces: Record<string, Record<string, MemberManifest>>;
  types: Record<string, { text: string }>;
};

export const API_MANIFEST = manifest as ApiManifest;

export const API_SURFACE = Object.freeze(Object.fromEntries(
  Object.entries(API_MANIFEST.interfaces).map(([name, members]) => [
    name,
    Object.keys(members),
  ]),
));

export function renderApiReference(browserType?: string, apiSupportOverrides: Record<string, boolean> = {}): string {
  const blocks: string[] = [];
  for (const [name, members] of Object.entries(API_MANIFEST.interfaces)) {
    const declarations = Object.entries(members).flatMap(([memberName, member]) => {
      if (member.documented === false) return [];
      if (browserType !== undefined) {
        const memberId = `${name}.${memberName}`;
        const supported = apiSupportOverrides[memberId]
          ?? member.unsupportedByDefaultIn?.includes(browserType) !== true;
        if (!supported) return [];
      }
      return member.declarations.flatMap((declaration) => {
        if (declaration.documented === false) return [];
        if (browserType !== undefined && declaration.unsupportedByDefaultIn?.includes(browserType) === true) return [];
        return [`  ${declaration.text}`];
      });
    });
    if (declarations.length > 0) blocks.push(`interface ${name} {\n${declarations.join("\n")}\n}`);
  }
  blocks.push(...Object.values(API_MANIFEST.types).map(({ text }) => text));
  return blocks.join("\n\n");
}

const DOCUMENTS: Record<string, string> = {
  "api-use-behavior": "Use the highest-level Browser API that completes the task. Locator reads and actions preserve browser ownership and provenance.",
  "all-tabs-cleanup": "Finalize only tabs owned by this caller. Never close unrelated user tabs.",
  "browser-control-interruption": "Browser commands are cancellable by the node_repl host; do not retry ambiguous mutations.",
  "browser-safety": "Use the selected browser surface and keep actions within the caller-owned tab group.",
  "browser-troubleshooting": "Re-list browsers and tabs, then report the structured backend error without changing providers implicitly.",
  confirmations: "Use the host elicitation surface when the browser backend reports confirmation is required.",
  "file-uploads": "Use waitForEvent('filechooser') and setFiles; IAB may truthfully report this unsupported.",
  playwright: "Locator objects are lazy, serializable descriptions executed by the shared browser scheduler.",
  screenshots: "Browser screenshot coordinates are CSS pixels. Screenshots are returned as Uint8Array values.",
  "session-naming": "Name extension sessions when useful so caller-owned tab groups remain identifiable.",
  "tab-claiming-chrome": "Claim an existing user tab explicitly before controlling it.",
  "tab-claiming-iab": "Only claim IAB tabs advertised as claimable by the host.",
  "tab-cleanup-chrome": "Use tabs.finalize with explicit handoff or deliverable dispositions.",
  "tab-cleanup-iab": "Use tabs.finalize with explicit handoff or deliverable dispositions.",
  visibility: "The visibility capability reports or changes whether the host presents the browser surface.",
  viewport: "The viewport capability changes CSS viewport dimensions and can reset host defaults.",
};

export function readDocument(name: string): string {
  if (name === "api") return renderApiReference();
  const document = DOCUMENTS[name];
  if (document === undefined) throw new Error(`Unknown Browser documentation: ${name}`);
  return document;
}

export const DOCUMENT_NAMES = Object.freeze(Object.keys(DOCUMENTS).sort());
