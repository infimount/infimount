import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

vi.mock("@codemirror/lint", () => ({
  lintGutter: () => "lint-gutter-extension",
  linter: (callback: (view: { state: { doc: { toString: () => string; length: number } } }) => unknown) => {
    callback({ state: { doc: { toString: () => "{}", length: 2 } } });
    callback({ state: { doc: { toString: () => "{", length: 1 } } });
    return "json-linter-extension";
  },
}));

vi.mock("@uiw/react-codemirror", () => ({
  default: ({
    value,
    editable,
    height,
    onChange,
  }: {
    value: string;
    editable: boolean;
    height: string;
    onChange: (value: string) => void;
  }) => (
    <textarea
      aria-label="JSON editor"
      data-height={height}
      readOnly={!editable}
      value={value}
      onChange={(event) => onChange(event.target.value)}
    />
  ),
}));

import { JsonCodeEditor } from "./JsonCodeEditor";

describe("JsonCodeEditor", () => {
  it("renders an editable JSON editor and forwards changes", () => {
    const onChange = vi.fn();

    render(<JsonCodeEditor value={'{"name":"Local"}'} onChange={onChange} minHeight="240px" />);

    const editor = screen.getByLabelText("JSON editor");
    expect(editor).toHaveValue('{"name":"Local"}');
    expect(editor).toHaveAttribute("data-height", "240px");
    expect(editor).not.toHaveAttribute("readonly");

    fireEvent.change(editor, { target: { value: '{"name":"Docs"}' } });
    expect(onChange).toHaveBeenCalledWith('{"name":"Docs"}');
  });

  it("renders read-only when requested", () => {
    render(<JsonCodeEditor value="{}" onChange={() => undefined} readOnly />);

    expect(screen.getByLabelText("JSON editor")).toHaveAttribute("readonly");
  });
});
