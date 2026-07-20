export type JsonRecord = Record<string, unknown>;

export type VerificationCheck = {
  id: string;
  status: "passed" | "failed" | "blocked";
  detail: string;
};

export type VerificationReport = {
  status: "passed" | "failed" | "blocked";
  root: string;
  checks: VerificationCheck[];
  blockers: string[];
};

export type VerifyCuaNodeOptions = {
  root: string;
  allowFixtureValues?: boolean;
  expectedTarget?: string;
  enforceLockPaths?: string[];
};

export type CuaNodeLockInspection = {
  path: string;
  kind: "runtime" | "native-assets" | "unknown";
  status: "passed" | "blocked" | "failed";
  sha256: string;
  blockers: string[];
  detail: string;
};
