import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { effortOptions, modelOptions, routeSelection, StatePanel, StatusBadge } from "./App";
import { ALL_STATUSES } from "./domain";
import type { ModelCatalog } from "./types";

const catalog: ModelCatalog = {
  source: "codex",
  warning: null,
  models: [
    {
      id: "gpt-6-astra",
      model: "gpt-6-astra",
      displayName: "GPT-6-Astra",
      description: "Frontier",
      defaultReasoningEffort: "low",
      supportedReasoningEfforts: ["low", "ultra"].map((reasoningEffort) => ({ reasoningEffort, description: reasoningEffort })),
      isDefault: true,
      upgrade: null,
    },
    {
      id: "gpt-6-luna",
      model: "gpt-6-luna",
      displayName: "GPT-6-Luna",
      description: "Fast",
      defaultReasoningEffort: "medium",
      supportedReasoningEfforts: ["low", "medium", "max"].map((reasoningEffort) => ({ reasoningEffort, description: reasoningEffort })),
      isDefault: false,
      upgrade: null,
    },
  ],
};

describe("dashboard state surfaces", () => {
  it.each(["loading", "empty", "stale", "error", "ready"] as const)(
    "renders the %s state explicitly",
    (state) => {
      const markup = renderToStaticMarkup(<StatePanel state={state} title={state} />);
      expect(markup).toContain(`data-state="${state}"`);
      expect(markup).toContain(state);
    },
  );

  it("renders every persisted task status without collapsing quota outcomes", () => {
    for (const status of ALL_STATUSES) {
      const markup = renderToStaticMarkup(<StatusBadge status={status} />);
      expect(markup).toContain(`status-${status}`);
      expect(markup).toContain(status.replaceAll("_", " "));
    }
  });
});

describe("Codex model catalog selectors", () => {
  it("exposes discovered models and only the selected model's efforts", () => {
    expect(modelOptions(catalog)).toEqual(["gpt-6-astra", "gpt-6-luna"]);
    expect(effortOptions(catalog, "gpt-6-astra")).toEqual(["low", "ultra"]);
    expect(effortOptions(catalog, "gpt-6-luna")).toEqual(["low", "medium", "max"]);
  });

  it("normalizes an unsupported effort to the model default", () => {
    expect(routeSelection(catalog, "gpt-6-luna", "ultra")).toEqual({
      model: "gpt-6-luna",
      effort: "medium",
    });
  });
});
