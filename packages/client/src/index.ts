// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// What Oikome's screens ask a node for (architecture v3 §13.4). Screens
// depend on `NodeClient` only. `HttpClient` implements it for the web app
// served by the owner's node (`oikome serve --web`); the browser node (on a
// phone) and the desktop app will bring their own implementations.

import type {
  AcceptInvite,
  AcceptedInvite,
  AccessItem,
  AddExamples,
  ApiError,
  Assistant,
  AssistantCollections,
  AssistantConnected,
  AssistantRef,
  ConnectAssistant,
  Borrowed,
  BorrowedJobs,
  FileItem,
  FileListing,
  FileRef,
  ListFiles,
  ViewInPlace,
  BorrowedJobsOf,
  BorrowedRef,
  Contact,
  ContactInput,
  DidRef,
  Empty,
  ErrorCode,
  InstallPlugin,
  Invite,
  InviteCode,
  InviteRef,
  Lending,
  LendingSaved,
  Network,
  ListAccess,
  ListShares,
  NewShare,
  NewSpace,
  NodeInfo,
  NodeName,
  Plugin,
  PluginCollections,
  PluginFolder,
  PluginRead,
  PluginRef,
  ReadFileRows,
  ReadRow,
  ReadRows,
  RecoveryKey,
  RemovedContact,
  RevokedShare,
  RunSql,
  Search,
  SearchResults,
  Share,
  ShareRef,
  ShareSpace,
  ShowFolder,
  SetLending,
  SetNetwork,
  UseBorrowed,
  DailyTotals,
  DailyTotalsOf,
  SpaceRef,
  SpaceSummary,
  SqlDone,
  StorageSummary,
  NewStorage,
  DefaultStorage,
  StorageRef,
  StartDropbox,
  FinishDropbox,
  DropboxApp,
  DropboxSignIn,
  StartGoogleDrive,
  GoogleApp,
  GoogleSignIn,
  HomeCards,
  SourceSummary,
  NewSource,
  PauseSource,
  SetSourceEvery,
  SourceRef,
  SourceRun,
  FoundPlace,
  FolderRef,
  FolderListing,
  TableSummary,
} from "@oikome/api";
import { tableFromIPC } from "apache-arrow";
import type { Table } from "apache-arrow";

export type * from "@oikome/api";

/** Rows of a table page or a SQL result. */
export interface RowsResult {
  table: Table;
  /** Whether the node cut the result off at its size limit. */
  truncated: boolean;
}

/** What `runSql` returns: rows, or what the last statement changed. */
export type SqlResult = ({ kind: "rows" } & RowsResult) | ({ kind: "done" } & SqlDone);

/** Everything a screen can ask a node for. */
export interface NodeClient {
  nodeInfo(): Promise<NodeInfo>;
  renameNode(request: NodeName): Promise<NodeInfo>;
  recoveryKey(): Promise<RecoveryKey>;
  confirmRecoveryKey(): Promise<NodeInfo>;
  listSpaces(): Promise<SpaceSummary[]>;
  createSpace(request: NewSpace): Promise<SpaceSummary>;
  dropSpace(request: SpaceRef): Promise<Empty>;
  listTables(request: SpaceRef): Promise<TableSummary[]>;
  /** Once, on a new data center: the examples, in `language` when they are written in it. */
  addExamples(request?: AddExamples): Promise<SpaceSummary[]>;
  listStorages(): Promise<StorageSummary[]>;
  addStorage(request: NewStorage): Promise<StorageSummary>;
  setDefaultStorage(request: DefaultStorage): Promise<StorageSummary[]>;
  removeStorage(request: StorageRef): Promise<StorageSummary[]>;
  /** The owner's Dropbox app this node remembers, by its App key. */
  dropboxApp(): Promise<DropboxApp>;
  /** Dropbox's sign-in page and pending ID; the page opens in another tab. */
  startDropbox(request: StartDropbox): Promise<DropboxSignIn>;
  /** Finish connecting with the authorization code Dropbox showed. */
  finishDropbox(request: FinishDropbox): Promise<StorageSummary>;
  /** The owner's Google app this node remembers, by its client ID. */
  googleApp(): Promise<GoogleApp>;
  /** Google's sign-in page for connecting the owner's Google Drive; the app goes there. */
  startGoogleDrive(request: StartGoogleDrive): Promise<GoogleSignIn>;
  setHomeCards(request: HomeCards): Promise<NodeInfo>;
  listSources(): Promise<SourceSummary[]>;
  addSource(request: NewSource): Promise<SourceSummary>;
  pullSource(request: SourceRef): Promise<SourceSummary>;
  pauseSource(request: PauseSource): Promise<SourceSummary>;
  setSourceEvery(request: SetSourceEvery): Promise<SourceSummary>;
  listSourceRuns(request: SourceRef): Promise<SourceRun[]>;
  removeSource(request: SourceRef): Promise<SourceSummary[]>;
  showSourceFiles(request: SourceRef): Promise<Empty>;
  findPlaces(): Promise<FoundPlace[]>;
  listFolders(request?: FolderRef): Promise<FolderListing>;
  readRows(request: ReadRows): Promise<RowsResult>;
  readRow(request: ReadRow): Promise<RowsResult>;
  search(request: Search): Promise<SearchResults>;
  runSql(request: RunSql): Promise<SqlResult>;
  listContacts(): Promise<Contact[]>;
  addContact(request: ContactInput): Promise<Contact>;
  renameContact(request: ContactInput): Promise<Contact>;
  removeContact(request: DidRef): Promise<RemovedContact>;
  shareSpace(request: ShareSpace): Promise<NewShare>;
  listShares(request?: ListShares): Promise<Share[]>;
  revokeShare(request: ShareRef): Promise<RevokedShare>;
  listInvites(): Promise<Invite[]>;
  addInvite(request: InviteCode): Promise<Invite>;
  acceptInvite(request: AcceptInvite): Promise<AcceptedInvite>;
  declineInvite(request: InviteRef): Promise<Invite[]>;
  lending(): Promise<Lending>;
  setLending(request: SetLending): Promise<LendingSaved>;
  /** How this computer is reached from outside its network, what for, and what the owner chose. */
  network(): Promise<Network>;
  setNetwork(request: SetNetwork): Promise<Network>;
  listBorrowed(): Promise<Borrowed[]>;
  stopBorrowing(request: BorrowedRef): Promise<Borrowed[]>;
  /** Send big summaries of collections shared with its owner there, or stop. */
  useBorrowed(request: UseBorrowed): Promise<Borrowed[]>;
  /** What was sent to a computer lent to you, and which collections may be. */
  borrowedJobs(request: BorrowedJobsOf): Promise<BorrowedJobs>;
  /** A table's totals by day, worked out here or on a computer lent to you. */
  dailyTotals(request: DailyTotalsOf): Promise<DailyTotals>;
  /** A collection that views a folder where it is; nothing is copied. */
  viewInPlace(request: ViewInPlace): Promise<SpaceSummary>;
  /** Read a folder viewed in place again now. */
  refreshSpace(request: SpaceRef): Promise<SpaceSummary>;
  /** One folder of a collection viewed in place, from its index. */
  listFiles(request: ListFiles): Promise<FileListing>;
  /** One file of a collection viewed in place, with all of a note's text. */
  readFile(request: FileRef): Promise<FileItem>;
  /** A page of one CSV, TSV, Parquet or JSON file, read where it is. */
  readFileRows(request: ReadFileRows): Promise<RowsResult>;
  /** Open a folder viewed in place, or reveal one of its files, on this computer. */
  showFolder(request: ShowFolder): Promise<Empty>;
  /** Where the app shows a photo's thumbnail or original from. */
  fileUrl(kind: "thumb" | "original", file: FileRef): string;
  listPlugins(): Promise<Plugin[]>;
  inspectPlugin(request: PluginFolder): Promise<Plugin>;
  installPlugin(request: InstallPlugin): Promise<Plugin>;
  setPluginCollections(request: PluginCollections): Promise<Plugin>;
  removePlugin(request: PluginRef): Promise<Plugin[]>;
  /** A collection's rows, read as the plugin; refused unless it may read them. */
  pluginRead(request: PluginRead): Promise<RowsResult>;
  listAccess(request?: ListAccess): Promise<AccessItem[]>;
  listAssistants(): Promise<Assistant[]>;
  /** Connect an assistant; the answer holds its token, shown this once. */
  connectAssistant(request: ConnectAssistant): Promise<AssistantConnected>;
  setAssistantCollections(request: AssistantCollections): Promise<Assistant>;
  endAssistantAccess(request: AssistantRef): Promise<Assistant>;
  removeAssistant(request: AssistantRef): Promise<Assistant[]>;
}

/** A failed call: the node's error code and message, or `network`. */
export class OikomeError extends Error {
  readonly code: ErrorCode | "network";
  /** The HTTP status; 0 when the node could not be reached. */
  readonly status: number;
  /** For a failure written for people, which one (`invite_ended`), so the app words it in its language. */
  readonly reason?: string;
  /** The values the reason's sentence names. */
  readonly params: Record<string, string>;

  constructor(
    code: ErrorCode | "network",
    message: string,
    status: number,
    reason?: string,
    params: Record<string, string> = {},
  ) {
    super(message);
    this.name = "OikomeError";
    this.code = code;
    this.status = status;
    this.reason = reason;
    this.params = params;
  }
}

export interface HttpClientOptions {
  /** Origin of the node's web API; the page's own origin when omitted. */
  baseUrl?: string;
  /** The fetch to use; the global one when omitted. */
  fetch?: typeof fetch;
}

const ARROW_STREAM = "application/vnd.apache.arrow.stream";

/** `NodeClient` over the node's web API: `POST /api/<operation>`. */
export class HttpClient implements NodeClient {
  readonly #base: string;
  readonly #fetch: typeof fetch;

  constructor(options: HttpClientOptions = {}) {
    this.#base = options.baseUrl ?? "";
    this.#fetch = options.fetch ?? globalThis.fetch.bind(globalThis);
  }

  async #post(operation: string, request: unknown): Promise<Response> {
    let response: Response;
    try {
      response = await this.#fetch(`${this.#base}/api/${operation}`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(request ?? {}),
        credentials: "same-origin",
      });
    } catch (e) {
      throw new OikomeError("network", e instanceof Error ? e.message : String(e), 0);
    }
    if (!response.ok) {
      let code: ErrorCode = "internal";
      let message = `the node answered ${response.status}`;
      let reason: string | undefined;
      let params: Record<string, string> = {};
      try {
        const error = (await response.json()) as ApiError;
        code = error.code;
        message = error.message;
        reason = error.reason;
        params = error.params ?? {};
      } catch {
        // Not a JSON error body.
      }
      throw new OikomeError(code, message, response.status, reason, params);
    }
    return response;
  }

  async #json<T>(operation: string, request: unknown): Promise<T> {
    const response = await this.#post(operation, request);
    return (await response.json()) as T;
  }

  async #rows(response: Response): Promise<RowsResult> {
    const bytes = new Uint8Array(await response.arrayBuffer());
    return {
      table: tableFromIPC(bytes),
      truncated: response.headers.get("oikome-truncated") === "true",
    };
  }

  nodeInfo(): Promise<NodeInfo> {
    return this.#json("node_info", {});
  }

  renameNode(request: NodeName): Promise<NodeInfo> {
    return this.#json("rename_node", request);
  }

  recoveryKey(): Promise<RecoveryKey> {
    return this.#json("recovery_key", {});
  }

  confirmRecoveryKey(): Promise<NodeInfo> {
    return this.#json("confirm_recovery_key", {});
  }

  listSpaces(): Promise<SpaceSummary[]> {
    return this.#json("list_spaces", {});
  }

  createSpace(request: NewSpace): Promise<SpaceSummary> {
    return this.#json("create_space", request);
  }

  dropSpace(request: SpaceRef): Promise<Empty> {
    return this.#json("drop_space", request);
  }

  listTables(request: SpaceRef): Promise<TableSummary[]> {
    return this.#json("list_tables", request);
  }

  addExamples(request: AddExamples = {}): Promise<SpaceSummary[]> {
    return this.#json("add_examples", request);
  }

  listStorages(): Promise<StorageSummary[]> {
    return this.#json("list_storages", {});
  }

  addStorage(request: NewStorage): Promise<StorageSummary> {
    return this.#json("add_storage", request);
  }

  setDefaultStorage(request: DefaultStorage): Promise<StorageSummary[]> {
    return this.#json("set_default_storage", request);
  }

  removeStorage(request: StorageRef): Promise<StorageSummary[]> {
    return this.#json("remove_storage", request);
  }

  dropboxApp(): Promise<DropboxApp> {
    return this.#json("dropbox_app", {});
  }

  startDropbox(request: StartDropbox): Promise<DropboxSignIn> {
    return this.#json("start_dropbox", request);
  }

  finishDropbox(request: FinishDropbox): Promise<StorageSummary> {
    return this.#json("finish_dropbox", request);
  }

  googleApp(): Promise<GoogleApp> {
    return this.#json("google_app", {});
  }

  startGoogleDrive(request: StartGoogleDrive): Promise<GoogleSignIn> {
    return this.#json("start_google_drive", request);
  }

  setHomeCards(request: HomeCards): Promise<NodeInfo> {
    return this.#json("set_home_cards", request);
  }

  listSources(): Promise<SourceSummary[]> {
    return this.#json("list_sources", {});
  }

  addSource(request: NewSource): Promise<SourceSummary> {
    return this.#json("add_source", request);
  }

  pullSource(request: SourceRef): Promise<SourceSummary> {
    return this.#json("pull_source", request);
  }

  pauseSource(request: PauseSource): Promise<SourceSummary> {
    return this.#json("pause_source", request);
  }

  setSourceEvery(request: SetSourceEvery): Promise<SourceSummary> {
    return this.#json("set_source_every", request);
  }

  listSourceRuns(request: SourceRef): Promise<SourceRun[]> {
    return this.#json("list_source_runs", request);
  }

  removeSource(request: SourceRef): Promise<SourceSummary[]> {
    return this.#json("remove_source", request);
  }

  showSourceFiles(request: SourceRef): Promise<Empty> {
    return this.#json("show_source_files", request);
  }

  findPlaces(): Promise<FoundPlace[]> {
    return this.#json("find_places", {});
  }

  listFolders(request: FolderRef = {}): Promise<FolderListing> {
    return this.#json("list_folders", request);
  }

  async readRows(request: ReadRows): Promise<RowsResult> {
    return this.#rows(await this.#post("read_rows", request));
  }

  async readRow(request: ReadRow): Promise<RowsResult> {
    return this.#rows(await this.#post("read_row", request));
  }

  search(request: Search): Promise<SearchResults> {
    return this.#json("search", request);
  }

  async runSql(request: RunSql): Promise<SqlResult> {
    const response = await this.#post("run_sql", request);
    if (response.headers.get("content-type")?.startsWith(ARROW_STREAM)) {
      return { kind: "rows", ...(await this.#rows(response)) };
    }
    return { kind: "done", ...((await response.json()) as SqlDone) };
  }

  listContacts(): Promise<Contact[]> {
    return this.#json("list_contacts", {});
  }

  addContact(request: ContactInput): Promise<Contact> {
    return this.#json("add_contact", request);
  }

  renameContact(request: ContactInput): Promise<Contact> {
    return this.#json("rename_contact", request);
  }

  removeContact(request: DidRef): Promise<RemovedContact> {
    return this.#json("remove_contact", request);
  }

  shareSpace(request: ShareSpace): Promise<NewShare> {
    return this.#json("share_space", request);
  }

  listShares(request: ListShares = {}): Promise<Share[]> {
    return this.#json("list_shares", request);
  }

  revokeShare(request: ShareRef): Promise<RevokedShare> {
    return this.#json("revoke_share", request);
  }

  listInvites(): Promise<Invite[]> {
    return this.#json("list_invites", {});
  }

  addInvite(request: InviteCode): Promise<Invite> {
    return this.#json("add_invite", request);
  }

  acceptInvite(request: AcceptInvite): Promise<AcceptedInvite> {
    return this.#json("accept_invite", request);
  }

  declineInvite(request: InviteRef): Promise<Invite[]> {
    return this.#json("decline_invite", request);
  }

  lending(): Promise<Lending> {
    return this.#json("lending", {});
  }

  setLending(request: SetLending): Promise<LendingSaved> {
    return this.#json("set_lending", request);
  }

  network(): Promise<Network> {
    return this.#json("network", {});
  }

  setNetwork(request: SetNetwork): Promise<Network> {
    return this.#json("set_network", request);
  }

  listBorrowed(): Promise<Borrowed[]> {
    return this.#json("list_borrowed", {});
  }

  stopBorrowing(request: BorrowedRef): Promise<Borrowed[]> {
    return this.#json("stop_borrowing", request);
  }

  useBorrowed(request: UseBorrowed): Promise<Borrowed[]> {
    return this.#json("use_borrowed", request);
  }

  borrowedJobs(request: BorrowedJobsOf): Promise<BorrowedJobs> {
    return this.#json("borrowed_jobs", request);
  }

  dailyTotals(request: DailyTotalsOf): Promise<DailyTotals> {
    return this.#json("daily_totals", request);
  }

  viewInPlace(request: ViewInPlace): Promise<SpaceSummary> {
    return this.#json("view_in_place", request);
  }

  refreshSpace(request: SpaceRef): Promise<SpaceSummary> {
    return this.#json("refresh_space", request);
  }

  listFiles(request: ListFiles): Promise<FileListing> {
    return this.#json("list_files", request);
  }

  readFile(request: FileRef): Promise<FileItem> {
    return this.#json("read_file", request);
  }

  async readFileRows(request: ReadFileRows): Promise<RowsResult> {
    return this.#rows(await this.#post("read_file_rows", request));
  }

  showFolder(request: ShowFolder): Promise<Empty> {
    return this.#json("show_folder", request);
  }

  fileUrl(kind: "thumb" | "original", file: FileRef): string {
    const path = file.path.split("/").map(encodeURIComponent).join("/");
    return `/files/${kind}/${encodeURIComponent(file.space)}/${path}`;
  }

  listPlugins(): Promise<Plugin[]> {
    return this.#json("list_plugins", {});
  }

  inspectPlugin(request: PluginFolder): Promise<Plugin> {
    return this.#json("inspect_plugin", request);
  }

  installPlugin(request: InstallPlugin): Promise<Plugin> {
    return this.#json("install_plugin", request);
  }

  setPluginCollections(request: PluginCollections): Promise<Plugin> {
    return this.#json("set_plugin_collections", request);
  }

  removePlugin(request: PluginRef): Promise<Plugin[]> {
    return this.#json("remove_plugin", request);
  }

  async pluginRead(request: PluginRead): Promise<RowsResult> {
    return this.#rows(await this.#post("plugin_read", request));
  }

  listAssistants(): Promise<Assistant[]> {
    return this.#json("list_assistants", {});
  }

  connectAssistant(request: ConnectAssistant): Promise<AssistantConnected> {
    return this.#json("connect_assistant", request);
  }

  setAssistantCollections(request: AssistantCollections): Promise<Assistant> {
    return this.#json("set_assistant_collections", request);
  }

  endAssistantAccess(request: AssistantRef): Promise<Assistant> {
    return this.#json("end_assistant_access", request);
  }

  removeAssistant(request: AssistantRef): Promise<Assistant[]> {
    return this.#json("remove_assistant", request);
  }

  listAccess(request: ListAccess = {}): Promise<AccessItem[]> {
    return this.#json("list_access", request);
  }
}
