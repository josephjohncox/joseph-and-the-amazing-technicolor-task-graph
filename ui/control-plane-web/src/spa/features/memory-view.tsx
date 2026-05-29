import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import clsx from "clsx";
import { Search } from "lucide-react";
import { useEffect, useState } from "react";

import {
  at,
  isRecord,
  memoryContext,
  memoryEdit,
  memoryEditPreview,
  memoryEvents,
  memorySearch,
  memoryWrite,
  rowsFrom,
} from "../api";
import { AdvancedInspect, EmptyState, ResultList, SimpleTable } from "../components/operator-primitives";
import type { JsonRecord } from "../types";
import { excerpt, friendlyRef, tokenList } from "./workbench-format";

export function MemoryView({ selectedGoalId }: { selectedGoalId: string }) {
  const queryClient = useQueryClient();
  const [query, setQuery] = useState("");
  const [note, setNote] = useState("");
  const [result, setResult] = useState<unknown>(null);
  const [replaceKeysText, setReplaceKeysText] = useState("");
  const [replacementKey, setReplacementKey] = useState("");
  const [replacementTitle, setReplacementTitle] = useState("");
  const [replacementContent, setReplacementContent] = useState("");
  const [replacementReason, setReplacementReason] = useState("");
  const [replacementTagsText, setReplacementTagsText] = useState("operator, reviewed");
  const [previewResult, setPreviewResult] = useState<unknown>(null);
  useEffect(() => {
    setPreviewResult(null);
  }, [selectedGoalId]);
  const memoryEventsQuery = useQuery({
    queryKey: ["memory-events", selectedGoalId],
    queryFn: () => memoryEvents(selectedGoalId),
    enabled: Boolean(selectedGoalId),
  });
  const editPayload = () => {
    if (!selectedGoalId) {
      throw new Error("Select a goal.");
    }
    return memoryEditPayload({
      goalId: selectedGoalId,
      replaceKeys: tokenList(replaceKeysText),
      replacementKey,
      replacementTitle,
      replacementContent,
      replacementReason,
      replacementTags: tokenList(replacementTagsText),
    });
  };
  const searchMutation = useMutation({
    mutationFn: () => memorySearch({ goal_id: selectedGoalId || undefined, query, limit: 8 }),
    onSuccess: setResult,
  });
  const contextMutation = useMutation({
    mutationFn: () => memoryContext({ goal_id: selectedGoalId || undefined, query, limit: 8 }),
    onSuccess: setResult,
  });
  const writeMutation = useMutation({
    mutationFn: () => memoryWrite({
      goal_id: selectedGoalId || undefined,
      scope: selectedGoalId ? "goal" : "global",
      kind: "operator_note",
      text: note,
      tags: ["operator", "dashboard"],
    }),
    onSuccess: (value) => {
      setResult(value);
      void queryClient.invalidateQueries({ queryKey: ["memory-events", selectedGoalId] });
    },
  });
  const previewMutation = useMutation({
    mutationFn: () => memoryEditPreview(editPayload()),
    onSuccess: setPreviewResult,
  });
  const editMutation = useMutation({
    mutationFn: () => memoryEdit({
      ...editPayload(),
      task_id: null,
      scope: "goal",
      store: null,
    }),
    onSuccess: (value) => {
      setResult(value);
      void queryClient.invalidateQueries({ queryKey: ["memory-events", selectedGoalId] });
    },
  });
  const busy = searchMutation.isPending || contextMutation.isPending || writeMutation.isPending || previewMutation.isPending || editMutation.isPending;
  const replacementReady = Boolean(
    selectedGoalId
      && tokenList(replaceKeysText).length
      && replacementTitle.trim()
      && replacementContent.trim()
      && replacementReason.trim(),
  );
  const editError = previewMutation.error ?? editMutation.error;
  return (
    <section className="memory-layout">
      <div className="panel-stack">
        <div className="panel">
          <div className="section-heading">
            <h2>Search shared memory</h2>
            <Search size={18} />
          </div>
          <label>
            Search or context request
            <textarea value={query} onChange={(event) => setQuery(event.target.value)} placeholder="What should the agents remember before continuing?" />
          </label>
          <div className="button-row">
            <button className="primary-button" type="button" disabled={busy} onClick={() => searchMutation.mutate()}>
              Search
            </button>
            <button className="secondary-button" type="button" disabled={busy} onClick={() => contextMutation.mutate()}>
              Build context
            </button>
          </div>
          <label>
            Durable operator note
            <textarea value={note} onChange={(event) => setNote(event.target.value)} placeholder="Write a reviewed fact, constraint, or decision." />
          </label>
          <button className="secondary-button" type="button" disabled={busy || !note.trim()} onClick={() => writeMutation.mutate()}>
            Save memory note
          </button>
        </div>

        <div className="panel">
          <div className="section-heading">
            <h2>Replace memory</h2>
            <span className={clsx("status-pill", selectedGoalId ? "status-running" : "status-pending")}>
              {selectedGoalId ? friendlyRef(selectedGoalId) : "Select goal"}
            </span>
          </div>
          <label>
            Replace keys
            <textarea
              value={replaceKeysText}
              onChange={(event) => {
                setReplaceKeysText(event.target.value);
                setPreviewResult(null);
              }}
              placeholder="memory-key-1, memory-key-2"
            />
          </label>
          <label>
            Replacement key
            <input
              value={replacementKey}
              onChange={(event) => {
                setReplacementKey(event.target.value);
                setPreviewResult(null);
              }}
              placeholder="optional stable key"
            />
          </label>
          <label>
            Replacement title
            <input
              value={replacementTitle}
              onChange={(event) => {
                setReplacementTitle(event.target.value);
                setPreviewResult(null);
              }}
              placeholder="Reviewed decision"
            />
          </label>
          <label>
            Replacement content
            <textarea
              value={replacementContent}
              onChange={(event) => {
                setReplacementContent(event.target.value);
                setPreviewResult(null);
              }}
              placeholder="Reviewed replacement memory."
            />
          </label>
          <label>
            Reason
            <input
              value={replacementReason}
              onChange={(event) => {
                setReplacementReason(event.target.value);
                setPreviewResult(null);
              }}
              placeholder="why the replacement supersedes the old keys"
            />
          </label>
          <label>
            Tags
            <input
              value={replacementTagsText}
              onChange={(event) => {
                setReplacementTagsText(event.target.value);
                setPreviewResult(null);
              }}
              placeholder="operator, reviewed"
            />
          </label>
          <div className="button-row">
            <button className="primary-button" type="button" disabled={busy || !replacementReady} onClick={() => previewMutation.mutate()}>
              Preview diff
            </button>
            <button className="secondary-button" type="button" disabled={busy || !replacementReady || !previewReady(previewResult)} onClick={() => editMutation.mutate()}>
              Apply edit
            </button>
          </div>
          {editError && <span className="error-text">{editError.message}</span>}
        </div>
      </div>
      <div className="panel-stack">
        <div className="panel">
          <div className="section-heading">
            <h2>Memory results</h2>
            <span className="muted-small">Scoped by goal when selected</span>
          </div>
          <ResultList value={result} />
        </div>
        <div className="panel">
          <div className="section-heading">
            <h2>Replacement diff</h2>
            <PreviewStatus value={previewResult} />
          </div>
          <MemoryDiffTable value={previewResult} />
        </div>
        <div className="panel">
          <div className="section-heading">
            <h2>Memory events</h2>
            {Boolean(memoryEventsQuery.data) && (
              <AdvancedInspect summaryLabel="Details" title="Memory events" payload={memoryEventsQuery.data} buttonLabel="Inspect JSON" />
            )}
          </div>
          <MemoryEventsTable selectedGoalId={selectedGoalId} value={memoryEventsQuery.data} loading={memoryEventsQuery.isFetching} />
        </div>
      </div>
    </section>
  );
}

export function memoryEditPayload(input: {
  goalId: string;
  replaceKeys: string[];
  replacementKey: string;
  replacementTitle: string;
  replacementContent: string;
  replacementReason: string;
  replacementTags: string[];
}): JsonRecord {
  return {
    goal_id: input.goalId,
    replace_keys: input.replaceKeys,
    replacement_key: input.replacementKey.trim() || null,
    replacement_episode: {
      title: input.replacementTitle.trim(),
      content: input.replacementContent.trim(),
      source: {
        source_type: "human",
        uri: null,
        actor: "operator",
      },
      artifacts: [],
      tags: input.replacementTags,
    },
    reason: input.replacementReason.trim(),
  };
}

export function PreviewStatus({ value }: { value: unknown }) {
  if (!value) {
    return <span className="status-pill muted">Preview pending</span>;
  }
  return (
    <span className={clsx("status-pill", previewReady(value) ? "status-done" : "status-blocked")}>
      {previewReady(value) ? "Ready" : "Blocked"}
    </span>
  );
}

export function previewReady(value: unknown): boolean {
  const record = previewRecord(value);
  return Boolean(record?.ready_to_edit);
}

function previewRecord(value: unknown): JsonRecord | null {
  const data = at(value, ["data"]);
  if (isRecord(data)) {
    return data;
  }
  return isRecord(value) ? value : null;
}

export function MemoryDiffTable({ value }: { value: unknown }) {
  const record = previewRecord(value);
  if (!record) {
    return <EmptyState title="Preview memory edit" detail="Choose replacement details." />;
  }
  const diffs = rowsFrom(record.diffs);
  const missingKeys = arrayStrings(record.missing_keys);
  return (
    <>
      <div className="summary-row">
        <span className="status-pill">Replacement {String(record.replacement_key ?? "auto key")}</span>
        {missingKeys.length > 0 && <span className="status-pill status-blocked">Missing {missingKeys.join(", ")}</span>}
      </div>
      <SimpleTable
        empty="Diff rows pending."
        headers={["Key", "Before", "After"]}
        rows={diffs.map((row) => [
          String(row.key ?? ""),
          titledExcerpt(row.before_title, row.before_excerpt),
          titledExcerpt(row.after_title, row.after_excerpt),
        ])}
      />
      <div className="summary-row">
        <AdvancedInspect summaryLabel="Details" title="Memory edit preview" payload={record} buttonLabel="Inspect JSON" />
      </div>
    </>
  );
}

export function MemoryEventsTable({ selectedGoalId, value, loading }: { selectedGoalId: string; value: unknown; loading: boolean }) {
  if (!selectedGoalId) {
    return <EmptyState title="Select a goal" detail="Memory events are scoped to the current goal." />;
  }
  if (loading && !value) {
    return <EmptyState title="Loading memory events" detail="Fetching memory event history." />;
  }
  const rows = rowsFrom(at(value, ["events"]) ?? value).slice(-10).reverse();
  return (
    <SimpleTable
      empty="Memory events pending."
      headers={["Action", "Key", "Scope", "Summary"]}
      rows={rows.map((row) => [
        String(row.action ?? ""),
        String(row.key ?? ""),
        String(row.scope ?? ""),
        excerpt(row.summary),
      ])}
    />
  );
}

function titledExcerpt(title: unknown, body: unknown): string {
  return [String(title ?? "").trim(), excerpt(body)].filter(Boolean).join(": ");
}

function arrayStrings(value: unknown): string[] {
  return Array.isArray(value) ? value.map(String).filter(Boolean) : [];
}
