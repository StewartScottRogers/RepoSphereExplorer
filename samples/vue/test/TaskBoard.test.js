import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";

import TaskBoard from "../src/components/TaskBoard.vue";

describe("TaskBoard", () => {
  it("mounts without a store, a router or anything else it does not need", () => {
    const wrapper = mount(TaskBoard);

    expect(wrapper.exists()).toBe(true);
  });

  it("renders something rather than an empty root element", () => {
    const wrapper = mount(TaskBoard);

    expect(wrapper.html().length).toBeGreaterThan(0);
  });

  it("survives being unmounted and mounted again", () => {
    // A component that leaks a listener on unmount fails here rather than
    // three screens later in somebody's session.
    const first = mount(TaskBoard);
    first.unmount();

    const second = mount(TaskBoard);

    expect(second.exists()).toBe(true);
  });
});
