import { beforeEach, describe, expect, it } from "vitest";

import {
  appendActivityLogEvent,
  clearActivityLogEvents,
  listActivityLogEvents,
} from "./activityLog";

describe("activityLog", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  it("persists, lists, and clears local activity events", () => {
    appendActivityLogEvent({
      type: "transfer_started",
      jobId: "transfer-1",
      operation: "copy",
      sourceId: "a",
      targetId: "b",
      pathCount: 2,
    });

    expect(listActivityLogEvents()).toMatchObject([
      {
        type: "transfer_started",
        jobId: "transfer-1",
        operation: "copy",
        sourceId: "a",
        targetId: "b",
        pathCount: 2,
      },
    ]);

    clearActivityLogEvents();
    expect(listActivityLogEvents()).toEqual([]);
  });
});
