// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// What Oikome's screens ask a node for (architecture v3 §13.4). Screens
// depend on `NodeClient` only. `HttpClient` implements it for the web app
// served by the owner's node (`oikome serve --web`); the browser node (on a
// phone) and the desktop app will bring their own implementations.

import type {
  AccessItem,
  ApiError,
  Contact,
  ContactInput,
  DidRef,
  Empty,
  ErrorCode,
  ListAccess,
  ListShares,
  NewShare,
  NewSpace,
  NodeInfo,
  NodeName,
  ReadRows,
  RecoveryKey,
  RemovedContact,
  RevokedShare,
  RunSql,
  Share,
  ShareRef,
  ShareSpace,
  SpaceRef,
  SpaceSummary,
  SqlDone,
  StorageSummary,
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
  addExamples(): Promise<SpaceSummary[]>;
  listStorages(): Promise<StorageSummary[]>;
  readRows(request: ReadRows): Promise<RowsResult>;
  runSql(request: RunSql): Promise<SqlResult>;
  listContacts(): Promise<Contact[]>;
  addContact(request: ContactInput): Promise<Contact>;
  renameContact(request: ContactInput): Promise<Contact>;
  removeContact(request: DidRef): Promise<RemovedContact>;
  shareSpace(request: ShareSpace): Promise<NewShare>;
  listShares(request?: ListShares): Promise<Share[]>;
  revokeShare(request: ShareRef): Promise<RevokedShare>;
  listAccess(request?: ListAccess): Promise<AccessItem[]>;
}

/** A failed call: the node's error code and message, or `network`. */
export class OikomeError extends Error {
  readonly code: ErrorCode | "network";
  /** The HTTP status; 0 when the node could not be reached. */
  readonly status: number;

  constructor(code: ErrorCode | "network", message: string, status: number) {
    super(message);
    this.name = "OikomeError";
    this.code = code;
    this.status = status;
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
      try {
        const error = (await response.json()) as ApiError;
        code = error.code;
        message = error.message;
      } catch {
        // Not a JSON error body.
      }
      throw new OikomeError(code, message, response.status);
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

  addExamples(): Promise<SpaceSummary[]> {
    return this.#json("add_examples", {});
  }

  listStorages(): Promise<StorageSummary[]> {
    return this.#json("list_storages", {});
  }

  async readRows(request: ReadRows): Promise<RowsResult> {
    return this.#rows(await this.#post("read_rows", request));
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

  listAccess(request: ListAccess = {}): Promise<AccessItem[]> {
    return this.#json("list_access", request);
  }
}
