// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// The persistent network entry: real state from the node, not another
// configuration surface or an unsupported claim of being online.

import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Button, Icon, Status } from "@oikome/ui";
import type { StatusKind } from "@oikome/ui";
import type { Network } from "@oikome/client";
import { useClient, useLoad } from "./data";
import { navigate } from "./router";
import { strings } from "./strings";

const s = strings.networkStatus;
const settings = strings.settings;

function stateOf(network: Network | undefined, loading: boolean): { kind: StatusKind; label: string } {
  if (!network) {
    return loading ? { kind: "run", label: strings.common.loading } : { kind: "off", label: s.unavailable };
  }
  if (!network.available) return { kind: "off", label: s.unavailable };
  switch (network.reachable) {
    case "direct":
      return { kind: "ok", label: settings.reachable.direct };
    case "through_contact":
      return {
        kind: "ok",
        label: settings.reachable.through_contact(network.through ?? strings.common.unknown),
      };
    case "opening":
      return { kind: "run", label: settings.reachable.opening };
    case "router_refused":
      return { kind: "bad", label: settings.reachable.router_refused };
    case "shared_address":
      return { kind: "bad", label: settings.reachable.shared_address };
    default:
      return { kind: "idle", label: settings.reachable.local_only };
  }
}

export function NetworkStatus({ className = "" }: { className?: string }) {
  const client = useClient();
  const network = useLoad(() => client.network(), []);
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const panel = useRef<HTMLDivElement>(null);
  const reload = network.reload;
  const state = stateOf(network.data, network.loading);

  useEffect(() => {
    const timer = window.setInterval(reload, 30_000);
    return () => window.clearInterval(timer);
  }, [reload]);

  useLayoutEffect(() => {
    const element = panel.current;
    const button = trigger.current;
    if (!open || !element || !button) return;
    element.showPopover?.();
    const place = () => {
      const at = button.getBoundingClientRect();
      const left = Math.min(at.left, window.innerWidth - element.offsetWidth - 8);
      element.style.left = `${Math.max(8, left)}px`;
      element.style.top = `${Math.min(at.bottom + 8, window.innerHeight - element.offsetHeight - 8)}px`;
    };
    place();
    window.addEventListener("resize", place);
    window.addEventListener("scroll", place, true);
    return () => {
      window.removeEventListener("resize", place);
      window.removeEventListener("scroll", place, true);
    };
  }, [open]);

  useEffect(() => {
    if (!open) return;
    reload();
    panel.current?.focus();
    const away = (event: PointerEvent) => {
      if (!root.current?.contains(event.target as Node)) setOpen(false);
    };
    const key = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        setOpen(false);
        trigger.current?.focus();
      }
    };
    document.addEventListener("pointerdown", away);
    document.addEventListener("keydown", key);
    return () => {
      document.removeEventListener("pointerdown", away);
      document.removeEventListener("keydown", key);
    };
  }, [open, reload]);

  const current = network.data;
  return (
    <div ref={root} className={`screen-network ${className}`}>
      <button
        ref={trigger}
        type="button"
        className="oik-icon-button screen-network__trigger"
        aria-label={`${s.title}: ${state.label}`}
        title={`${s.title}: ${state.label}`}
        aria-expanded={open}
        aria-controls={open ? "network-status-panel" : undefined}
        onClick={() => setOpen((value) => !value)}
      >
        <Icon name="globe" />
        <i className={`screen-network__dot screen-network__dot--${state.kind}`} aria-hidden="true" />
      </button>
      {open && (
        <div
          ref={panel}
          id="network-status-panel"
          className="screen-network__panel"
          popover="manual"
          role="dialog"
          aria-label={s.title}
          tabIndex={-1}
        >
          <div className="screen-network__head">
            <strong>{s.title}</strong>
            <Status kind={state.kind}>{state.label}</Status>
          </div>
          {network.error && <p className="screen-help screen-help--error">{network.error}</p>}
          {current && (
            <dl className="screen-network__facts">
              <div>
                <dt>{settings.openForLabel}</dt>
                <dd>
                  {current.openFor.length === 0
                    ? settings.openForNothing
                    : current.openFor.map((item) => (
                        <span key={`${item.kind}:${item.id}`}>
                          {item.kind === "lend"
                            ? settings.openForLend(item.name)
                            : settings.openForShare(item.space ?? "", item.kind === "public" ? strings.publicShare.anyone : item.name)}
                        </span>
                      ))}
                  {current.helping && <span>{settings.helpContacts}</span>}
                </dd>
              </div>
              <div>
                <dt>{s.addressLabel}</dt>
                <dd className="screen-network__address" translate="no" title={current.address ?? undefined}>
                  {current.address ?? s.noAddress}
                </dd>
              </div>
            </dl>
          )}
          <Button
            onClick={() => {
              setOpen(false);
              navigate("/settings?network=1");
            }}
          >
            {s.openSettings}
          </Button>
        </div>
      )}
    </div>
  );
}
