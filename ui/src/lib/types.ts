export type Protocol = "mqtt" | "mqtts" | "ws" | "wss";

export type FileKind = "ca" | "cert" | "key";

export type ProfileSummary = {
  id: string;
  name: string;
  protocol: Protocol;
  host: string;
  port: number;
};

export type TlsConfig = {
  validate: boolean;
  caCertPath: string | null;
  clientCertPath: string | null;
  clientKeyPath: string | null;
  alpn: string[] | null;
};

export type SessionConfig = {
  clean: boolean;
  keepAliveSecs: number;
  maxPacketSize: number;
};

export type Subscription = {
  topic: string;
  qos: number;
};

export type LastWill = {
  topic: string;
  payload: string;
  qos: number;
  retain: boolean;
};

export type ConnectionProfile = {
  id: string;
  name: string;
  protocol: Protocol;
  host: string;
  port: number;
  clientId: string;
  username: string;
  tls: TlsConfig;
  session: SessionConfig;
  subscriptions: Subscription[];
  lastWill: LastWill | null;
  websocketPath?: string | null;
  hasPassword: boolean;
};

export type SaveProfileInput = Omit<ConnectionProfile, "hasPassword"> & {
  password?: string;
};

export type ProfileDraft = {
  id: string;
  name: string;
  protocol: Protocol;
  host: string;
  port: number;
  clientId: string;
  username: string;
  password: string;
  hasPassword: boolean;
  caCertPath: string;
  clientCertPath: string;
  clientKeyPath: string;
  tlsValidate: boolean;
  alpn: string[] | null;
  session: SessionConfig;
  subscriptions: Subscription[];
  lastWill: LastWill | null;
  websocketPath: string | null;
};

const LAST_USED_KEY = "mqx:lastUsedProfileId";

const ADJECTIVES = [
  "quiet",
  "swift",
  "bold",
  "calm",
  "bright",
  "clever",
  "eager",
  "gentle",
  "lucky",
  "nimble",
  "proud",
  "rapid",
  "silent",
  "witty",
  "brave",
  "crisp",
  "mellow",
  "amber",
  "copper",
  "silver",
] as const;

const NOUNS = [
  "otter",
  "fox",
  "hawk",
  "lynx",
  "wren",
  "badger",
  "heron",
  "maple",
  "cedar",
  "pebble",
  "ember",
  "comet",
  "river",
  "willow",
  "sparrow",
  "falcon",
  "orchid",
  "cinder",
  "harbor",
  "meadow",
] as const;

export const DEFAULT_PROFILE: ProfileSummary = {
  id: "",
  name: "localhost",
  protocol: "mqtt",
  host: "localhost",
  port: 1883,
};

export function emptyLastWill(): LastWill {
  return { topic: "", payload: "", qos: 0, retain: false };
}

export function emptySubscription(): Subscription {
  return { topic: "#", qos: 0 };
}

export function generateClientId(avoid?: string): string {
  for (let attempt = 0; attempt < 8; attempt += 1) {
    const next = `mqx-${pick(ADJECTIVES)}-${pick(NOUNS)}`;
    if (next !== avoid) {
      return next;
    }
  }
  return `mqx-${pick(ADJECTIVES)}-${pick(NOUNS)}-${Math.floor(Math.random() * 90 + 10)}`;
}

export function emptyDraft(): ProfileDraft {
  return {
    id: "",
    name: "",
    protocol: DEFAULT_PROFILE.protocol,
    host: DEFAULT_PROFILE.host,
    port: DEFAULT_PROFILE.port,
    clientId: generateClientId(),
    username: "",
    password: "",
    hasPassword: false,
    caCertPath: "",
    clientCertPath: "",
    clientKeyPath: "",
    tlsValidate: true,
    alpn: null,
    session: {
      clean: false,
      keepAliveSecs: 60,
      maxPacketSize: 10_000_000,
    },
    subscriptions: [emptySubscription()],
    lastWill: null,
    websocketPath: null,
  };
}

export function draftFromProfile(profile: ConnectionProfile): ProfileDraft {
  return {
    id: profile.id,
    name: profile.name,
    protocol: profile.protocol,
    host: profile.host,
    port: profile.port,
    clientId: profile.clientId,
    username: profile.username,
    password: "",
    hasPassword: profile.hasPassword,
    caCertPath: profile.tls.caCertPath ?? "",
    clientCertPath: profile.tls.clientCertPath ?? "",
    clientKeyPath: profile.tls.clientKeyPath ?? "",
    tlsValidate: profile.tls.validate,
    alpn: profile.tls.alpn,
    session: { ...profile.session },
    subscriptions:
      profile.subscriptions.length > 0
        ? profile.subscriptions.map((subscription) => ({ ...subscription }))
        : [emptySubscription()],
    lastWill: profile.lastWill ? { ...profile.lastWill } : null,
    websocketPath: profile.websocketPath ?? null,
  };
}

export function draftToSaveInput(draft: ProfileDraft): SaveProfileInput {
  const subscriptions = draft.subscriptions
    .map((subscription) => ({
      topic: subscription.topic.trim(),
      qos: clampQos(subscription.qos),
    }))
    .filter((subscription) => subscription.topic.length > 0);
  const lastWill =
    draft.lastWill && draft.lastWill.topic.trim()
      ? {
          topic: draft.lastWill.topic.trim(),
          payload: draft.lastWill.payload,
          qos: clampQos(draft.lastWill.qos),
          retain: draft.lastWill.retain,
        }
      : null;
  const input: SaveProfileInput = {
    id: draft.id,
    name: draft.name.trim() || draft.host.trim() || DEFAULT_PROFILE.name,
    protocol: draft.protocol,
    host: draft.host.trim() || DEFAULT_PROFILE.host,
    port: clampPort(draft.port, defaultPort(draft.protocol)),
    clientId: draft.clientId.trim(),
    username: draft.username.trim(),
    tls: {
      validate: draft.tlsValidate,
      caCertPath: draft.caCertPath.trim() || null,
      clientCertPath: draft.clientCertPath.trim() || null,
      clientKeyPath: draft.clientKeyPath.trim() || null,
      alpn: draft.alpn,
    },
    session: {
      clean: draft.session.clean,
      keepAliveSecs: Math.max(1, asInt(draft.session.keepAliveSecs, 60)),
      maxPacketSize: Math.max(1, asInt(draft.session.maxPacketSize, 10_000_000)),
    },
    subscriptions,
    lastWill,
    websocketPath: draft.websocketPath,
  };
  if (draft.password !== "") {
    input.password = draft.password;
  }
  return input;
}

export function defaultPort(protocol: Protocol): number {
  switch (protocol) {
    case "mqtt":
      return 1883;
    case "mqtts":
      return 8883;
    case "ws":
      return 8083;
    case "wss":
      return 8084;
  }
}

export function brokerLabel(profile: Pick<ProfileSummary, "protocol" | "host" | "port">): string {
  return `${profile.protocol}://${profile.host}:${profile.port}`;
}

export function getLastUsedProfileId(): string | null {
  try {
    return localStorage.getItem(LAST_USED_KEY);
  } catch {
    return null;
  }
}

export function setLastUsedProfileId(id: string | null): void {
  try {
    if (id) {
      localStorage.setItem(LAST_USED_KEY, id);
    } else {
      localStorage.removeItem(LAST_USED_KEY);
    }
  } catch {
    // Private mode / disabled storage should not block the picker.
  }
}

export function duplicateName(name: string): string {
  return name.endsWith(" (copy)") ? name : `${name} (copy)`;
}

export type SessionStatusKind =
  | "connecting"
  | "connected"
  | "detached"
  | "reconnecting"
  | "disconnected"
  | "error";

export function sessionOpen(status: SessionStatusKind): boolean {
  return (
    status === "connecting" ||
    status === "connected" ||
    status === "reconnecting" ||
    status === "detached"
  );
}

export function statusLabel(status: SessionStatusKind): string {
  switch (status) {
    case "connected":
      return "Live";
    case "detached":
      return "Detached";
    default:
      return status;
  }
}

export type SessionStatus = {
  profileId: string | null;
  epoch?: number;
  status: SessionStatusKind;
  error?: string;
  broker: string;
  ramExhausted?: boolean;
};

export type SessionStats = {
  profileId?: string;
  epoch?: number;
  topics: number;
  messagesTotal: number;
  messagesPerSec: number;
  storedBytes?: number;
  ramLimitBytes?: number;
};

export type PayloadFormat = "json" | "text" | "binary";

export type TreeNodeDto = {
  segment: string;
  path: string;
  childCount: number;
  hasPayload: boolean;
  retain: boolean;
  freshness: "fresh" | "intime" | "stale" | "retain";
  format: PayloadFormat;
  lastMs: number;
  historyLen: number;
};

export type MessageDto = {
  epoch?: number;
  topic: string;
  payloadText: string;
  payloadJson?: unknown;
  format: PayloadFormat;
  retain: boolean;
  qos: 0 | 1 | 2;
  timestamp: number;
  size: number;
  error?: string;
};

export type TreeBatch = {
  profileId?: string;
  epoch?: number;
  upserts: TreeNodeDto[];
  deletes: string[];
};

export type SearchMode = "keep" | "skip";

export type SearchHit = {
  path: string;
  highlights: number[];
};

export type JqError = {
  message: string;
  start: number;
  end: number;
};

export type JqApplyResult = {
  text: string;
  json?: unknown;
  errors: JqError[];
};

export type HistoryMeta = {
  count: number;
  latestIndex: number;
};

export type HistoryItem = {
  index: number;
  timestamp: number;
  format: PayloadFormat;
  retain: boolean;
  qos: 0 | 1 | 2;
  size: number;
};

export type UiSettings = {
  theme: "dark" | "light" | "system";
  ramLimitBytes: number;
};

export const DEFAULT_RAM_LIMIT_BYTES = 12 * 1024 * 1024 * 1024;
export const RAM_LIMIT_MIN_GB = 4;
export const RAM_LIMIT_MAX_GB = 128;
const GIB = 1024 * 1024 * 1024;

export function idleSessionStatus(): SessionStatus {
  return {
    profileId: null,
    epoch: 0,
    status: "disconnected",
    broker: "",
  };
}

export function formatRate(messagesPerSec: number): string {
  if (messagesPerSec >= 1000) {
    return `${(messagesPerSec / 1000).toFixed(1)}k/s`;
  }
  return `${messagesPerSec.toFixed(messagesPerSec >= 10 ? 0 : 1)}/s`;
}

function formatGib(bytes: number): string {
  const gb = bytes / GIB;
  if (Math.abs(gb - Math.round(gb)) < 1e-9) {
    return String(Math.round(gb));
  }
  return gb >= 10 ? gb.toFixed(0) : gb.toFixed(1);
}

export function formatStoreUsage(storedBytes: number, limitBytes: number): string | null {
  if (!(limitBytes > 0) || storedBytes * 2 < limitBytes) {
    return null;
  }
  return `${formatGib(storedBytes)} / ${formatGib(limitBytes)} GB`;
}

export function ramLimitGb(bytes: number): number {
  return Math.round(bytes / GIB);
}

export function errorMessage(err: unknown): string {
  if (typeof err === "string") {
    return err;
  }
  if (err instanceof Error) {
    return err.message;
  }
  return String(err);
}

function pick<T extends readonly string[]>(values: T): T[number] {
  return values[Math.floor(Math.random() * values.length)] ?? values[0];
}

function clampQos(qos: number): number {
  const value = asInt(qos, 0);
  return value === 1 || value === 2 ? value : 0;
}

function asInt(value: number, fallback: number): number {
  const parsed = Number(value);
  return Number.isFinite(parsed) ? Math.trunc(parsed) : fallback;
}

function clampPort(value: number, fallback: number): number {
  const parsed = asInt(value, fallback);
  return parsed < 1 || parsed > 65535 ? fallback : parsed;
}
