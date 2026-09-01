import { invoke } from "@tauri-apps/api/core";
import type {
  ConnectionProfile,
  FileKind,
  HistoryItem,
  HistoryMeta,
  JqApplyResult,
  MessageDto,
  ProfileSummary,
  SaveProfileInput,
  SearchHit,
  SearchMode,
  RecordStatus,
  RecordingsList,
  ReplayReply,
  SessionStatus,
  StoppedRecording,
  TreeNodeDto,
  UiSettings,
} from "./types";
import type { Theme } from "./theme";

export function listProfiles(): Promise<ProfileSummary[]> {
  return invoke<ProfileSummary[]>("listProfiles");
}

export function getProfile(id: string): Promise<ConnectionProfile> {
  return invoke<ConnectionProfile>("getProfile", { id });
}

export function saveProfile(profile: SaveProfileInput): Promise<{ id: string }> {
  return invoke<{ id: string }>("saveProfile", { ...profile });
}

export function deleteProfile(id: string): Promise<"ok"> {
  return invoke<"ok">("deleteProfile", { id });
}

export function pickFile(kind: FileKind): Promise<{ path: string }> {
  return invoke<{ path: string }>("pickFile", { kind });
}

export function connect(id: string): Promise<{ status: "connecting"; epoch: number }> {
  return invoke<{ status: "connecting"; epoch: number }>("connect", { id });
}

export function disconnect(): Promise<{ status: "disconnected"; epoch: number }> {
  return invoke<{ status: "disconnected"; epoch: number }>("disconnect");
}

export function setIngest(enabled: boolean): Promise<SessionStatus> {
  return invoke<SessionStatus>("setIngest", { enabled });
}

export function listRecordings(): Promise<RecordingsList> {
  return invoke<RecordingsList>("listRecordings");
}

export function startReplay(path: string, profileId?: string | null): Promise<ReplayReply> {
  return invoke<ReplayReply>("startReplay", {
    path,
    profileId: profileId || null,
  });
}

export function stopReplay(): Promise<"ok"> {
  return invoke<"ok">("stopReplay");
}

export function treeChildren(path: string[]): Promise<TreeNodeDto[]> {
  return invoke<TreeNodeDto[]>("treeChildren", { path });
}

export function selectTopic(topic: string | null): Promise<"ok"> {
  return invoke<"ok">("selectTopic", { topic });
}

export function getMessage(topic: string, index: number | null): Promise<MessageDto> {
  return invoke<MessageDto>("getMessage", { topic, index });
}

export function getHistoryMeta(topic: string): Promise<HistoryMeta> {
  return invoke<HistoryMeta>("getHistoryMeta", { topic });
}

export function listHistory(topic: string): Promise<HistoryItem[]> {
  return invoke<HistoryItem[]>("listHistory", { topic });
}

export function treeSearch(query: string, mode: SearchMode): Promise<SearchHit[]> {
  return invoke<SearchHit[]>("treeSearch", { query, mode });
}

export function applyJq(
  topic: string,
  index: number | null,
  filter: string,
): Promise<JqApplyResult> {
  return invoke<JqApplyResult>("applyJq", { topic, index, filter });
}

export function jqHistory(topic: string): Promise<string[]> {
  return invoke<string[]>("jqHistory", { topic });
}

export function getSettings(): Promise<UiSettings> {
  return invoke<UiSettings>("getSettings");
}

export function setTheme(theme: Theme): Promise<UiSettings> {
  return invoke<UiSettings>("setTheme", { theme });
}

export function setRamLimit(bytes: number): Promise<UiSettings> {
  return invoke<UiSettings>("setRamLimit", { bytes });
}

export function setRecordDirectory(directory: string): Promise<UiSettings> {
  return invoke<UiSettings>("setRecordDirectory", { directory });
}

export function pickFolder(): Promise<{ path: string }> {
  return invoke<{ path: string }>("pickFolder");
}

export function startRecording(): Promise<RecordStatus> {
  return invoke<RecordStatus>("startRecording");
}

export function stopRecording(): Promise<StoppedRecording> {
  return invoke<StoppedRecording>("stopRecording");
}

export function saveRecording(tempPath: string, name: string): Promise<{ path: string }> {
  return invoke<{ path: string }>("saveRecording", { tempPath, name });
}

export function discardRecording(tempPath: string): Promise<"ok"> {
  return invoke<"ok">("discardRecording", { tempPath });
}

export function exitApp(): Promise<void> {
  return invoke<void>("exitApp");
}
