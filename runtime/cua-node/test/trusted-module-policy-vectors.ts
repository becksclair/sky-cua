export interface DescriptorRealPathVector {
  readonly name: string;
  readonly platform: NodeJS.Platform;
  readonly fd: number;
  readonly originalPath: string;
  readonly realpaths: Readonly<Record<string, string>>;
  readonly expectedCalls: readonly string[];
  readonly expected: string | null;
}

export const DESCRIPTOR_REALPATH_VECTORS: readonly DescriptorRealPathVector[] =
  [
    {
      name: "linux proc descriptor",
      platform: "linux",
      fd: 7,
      originalPath: "/configured/module.mjs",
      realpaths: { "/proc/self/fd/7": "/trusted/module.mjs" },
      expectedCalls: ["/proc/self/fd/7"],
      expected: "/trusted/module.mjs",
    },
    {
      name: "darwin falls through descriptor namespaces",
      platform: "darwin",
      fd: 8,
      originalPath: "/configured/module.mjs",
      realpaths: { "/proc/self/fd/8": "/trusted/module.mjs" },
      expectedCalls: ["/dev/fd/8", "/proc/self/fd/8"],
      expected: "/trusted/module.mjs",
    },
    {
      name: "windows falls back to the original path",
      platform: "win32",
      fd: 9,
      originalPath: "C:\\configured\\module.mjs",
      realpaths: { "C:\\configured\\module.mjs": "C:\\trusted\\module.mjs" },
      expectedCalls: [
        "/dev/fd/9",
        "/proc/self/fd/9",
        "C:\\configured\\module.mjs",
      ],
      expected: "C:\\trusted\\module.mjs",
    },
    {
      name: "descriptor lookup fails closed",
      platform: "linux",
      fd: 10,
      originalPath: "/configured/module.mjs",
      realpaths: {},
      expectedCalls: ["/proc/self/fd/10"],
      expected: null,
    },
  ];

export interface RootResolutionVector {
  readonly name: string;
  readonly paths: readonly string[];
  readonly realpaths: Readonly<Record<string, string>>;
  readonly directories: readonly string[];
  readonly expected: readonly string[];
}

export const ROOT_RESOLUTION_VECTORS: readonly RootResolutionVector[] = [
  {
    name: "roots resolve and retain only directories",
    paths: ["/configured-a", "/configured-file", "/missing", "/configured-b"],
    realpaths: {
      "/configured-a": "/real/a",
      "/configured-file": "/real/file",
      "/configured-b": "/real/b",
    },
    directories: ["/real/a", "/real/b"],
    expected: ["/real/a", "/real/b"],
  },
];

export interface TrustedFileAcquisitionVector {
  readonly name: string;
  readonly roots: readonly string[];
  readonly descriptorPaths: Readonly<Record<string, string>>;
  readonly fileDescriptors: readonly number[];
  readonly bytes: Readonly<Record<string, readonly number[]>>;
  readonly expected: readonly number[] | null;
  readonly expectedClosed: readonly number[];
}

export const TRUSTED_FILE_ACQUISITION_VECTORS: readonly TrustedFileAcquisitionVector[] =
  [
    {
      name: "containment falls through roots and returns descriptor bytes",
      roots: ["/trusted-a", "/trusted-b"],
      descriptorPaths: {
        "1": "/outside/module.mjs",
        "2": "/trusted-b/module.mjs",
      },
      fileDescriptors: [1, 2],
      bytes: { "2": [0, 255, 17] },
      expected: [0, 255, 17],
      expectedClosed: [1, 2],
    },
    {
      name: "non-file descriptors fail closed",
      roots: ["/trusted"],
      descriptorPaths: { "3": "/trusted/directory" },
      fileDescriptors: [],
      bytes: {},
      expected: null,
      expectedClosed: [3],
    },
  ];

export interface PolicyAction {
  readonly kind: "evaluate" | "probe" | "set-file";
  readonly path: string;
  readonly bytes?: readonly number[] | null;
  readonly inheritedTrust?: boolean;
  readonly expected?: boolean;
}

export interface PolicyParityVector {
  readonly name: string;
  readonly trustAllCode?: boolean;
  readonly initialFiles?: Readonly<Record<string, readonly number[]>>;
  readonly actions: readonly PolicyAction[];
}

export const POLICY_PARITY_VECTORS: readonly PolicyParityVector[] = [
  {
    name: "no grant fails closed",
    actions: [
      { kind: "evaluate", path: "/module.mjs", bytes: [1, 2], expected: false },
    ],
  },
  {
    name: "inherited trust grants trust",
    actions: [
      {
        kind: "evaluate",
        path: "/module.mjs",
        bytes: [3],
        inheritedTrust: true,
        expected: true,
      },
    ],
  },
  {
    name: "trust-all grants trust",
    trustAllCode: true,
    actions: [
      { kind: "evaluate", path: "/module.mjs", bytes: [4], expected: true },
    ],
  },
  {
    name: "direct directory read requires exact bytes",
    initialFiles: { "/trusted.mjs": [5, 6] },
    actions: [
      { kind: "evaluate", path: "/trusted.mjs", bytes: [5, 6], expected: true },
      {
        kind: "evaluate",
        path: "/trusted.mjs",
        bytes: [5, 7],
        expected: false,
      },
      { kind: "evaluate", path: "/trusted.mjs", bytes: [5], expected: false },
    ],
  },
  {
    name: "probe snapshots bytes and consumes the snapshot once",
    initialFiles: { "/trusted.mjs": [7, 8] },
    actions: [
      { kind: "probe", path: "/trusted.mjs", expected: true },
      { kind: "set-file", path: "/trusted.mjs", bytes: [9, 10] },
      { kind: "evaluate", path: "/trusted.mjs", bytes: [7, 8], expected: true },
      {
        kind: "evaluate",
        path: "/trusted.mjs",
        bytes: [7, 8],
        expected: false,
      },
      {
        kind: "evaluate",
        path: "/trusted.mjs",
        bytes: [9, 10],
        expected: true,
      },
    ],
  },
  {
    name: "snapshot rejects swapped loader bytes",
    initialFiles: { "/trusted.mjs": [11, 12] },
    actions: [
      { kind: "probe", path: "/trusted.mjs", expected: true },
      { kind: "set-file", path: "/trusted.mjs", bytes: [13, 14] },
      {
        kind: "evaluate",
        path: "/trusted.mjs",
        bytes: [13, 14],
        expected: false,
      },
    ],
  },
  {
    name: "failed probe clears an older snapshot",
    initialFiles: { "/trusted.mjs": [15] },
    actions: [
      { kind: "probe", path: "/trusted.mjs", expected: true },
      { kind: "set-file", path: "/trusted.mjs", bytes: null },
      { kind: "probe", path: "/trusted.mjs", expected: false },
      { kind: "evaluate", path: "/trusted.mjs", bytes: [15], expected: false },
    ],
  },
];
