import * as Dialog from "@radix-ui/react-dialog";
import { CircleAlert } from "lucide-react";

import { at, rowsFrom } from "../api";

export function ResultList({ value }: { value: unknown }) {
  const rows = rowsFrom(at(value, ["data"]) ?? value);
  if (!value) {
    return <EmptyState title="Memory results pending" detail="Search, build context, or save a note." />;
  }
  if (rows.length) {
    return (
      <ul className="result-list">
        {rows.slice(0, 20).map((row, index) => (
          <li key={String(row.key ?? row.id ?? index)}>
            <strong>{String(row.title ?? row.key ?? row.id ?? "Memory item")}</strong>
            <p>{String(row.content ?? row.text ?? row.summary ?? row.excerpt ?? "")}</p>
          </li>
        ))}
      </ul>
    );
  }
  return <AdvancedInspect summaryLabel="Details" title="Memory response" payload={value} buttonLabel="Inspect JSON" />;
}

export function SimpleTable({ headers, rows, empty }: { headers: string[]; rows: string[][]; empty: string }) {
  if (!rows.length) {
    return <EmptyState title={empty} detail="Refresh or connect the backing service." />;
  }
  return (
    <div className="table-wrap">
      <table>
        <thead>
          <tr>{headers.map((header) => <th key={header}>{header}</th>)}</tr>
        </thead>
        <tbody>
          {rows.map((row, index) => (
            <tr key={index}>{row.map((cell, cellIndex) => <td key={cellIndex}>{cell}</td>)}</tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

export function EmptyState({ title, detail, actionLabel, onAction }: { title: string; detail: string; actionLabel?: string; onAction?: () => void }) {
  return (
    <div className="empty-state">
      <CircleAlert size={18} />
      <strong>{title}</strong>
      <span>{detail}</span>
      {actionLabel && onAction && (
        <button type="button" className="secondary-button" onClick={onAction}>
          {actionLabel}
        </button>
      )}
    </div>
  );
}

export function AdvancedInspect({
  title,
  payload,
  buttonLabel = "Inspect",
  summaryLabel = "Advanced details",
}: {
  title: string;
  payload: unknown;
  buttonLabel?: string;
  summaryLabel?: string;
}) {
  return (
    <details className="advanced-inline-details">
      <summary>{summaryLabel}</summary>
      <div className="button-row">
        <InspectButton title={title} payload={payload} buttonLabel={buttonLabel} />
      </div>
    </details>
  );
}

export function InspectButton({ title, payload, buttonLabel = "Inspect" }: { title: string; payload: unknown; buttonLabel?: string }) {
  return (
    <Dialog.Root>
      <Dialog.Trigger asChild>
        <button type="button" className="secondary-button">{buttonLabel}</button>
      </Dialog.Trigger>
      <Dialog.Portal>
        <Dialog.Overlay className="dialog-overlay" />
        <Dialog.Content className="dialog-content">
          <Dialog.Title>{title}</Dialog.Title>
          <pre>{JSON.stringify(payload, null, 2)}</pre>
          <Dialog.Close asChild>
            <button type="button" className="primary-button">Close</button>
          </Dialog.Close>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
