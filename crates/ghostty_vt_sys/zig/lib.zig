const std = @import("std");
const ghostty_input = @import("ghostty_src/input.zig");
const terminal = @import("ghostty_src/terminal/main.zig");
const csi = @import("ghostty_src/terminal/csi.zig");

const Allocator = std.mem.Allocator;

// Grid event discriminator bytes returned by
// `ghostty_vt_terminal_take_grid_events`. Each event is encoded as a
// 2-byte record `[tag, reserved]`. Keep these in sync with
// `GridEventTag` on the Rust side (`crates/ghostty_vt/src/lib.rs`).
const GRID_EVENT_ALT_SCREEN_ENTER: u8 = 0;
const GRID_EVENT_ALT_SCREEN_EXIT: u8 = 1;
const GRID_EVENT_RIS: u8 = 2;

const TerminalHandle = struct {
    alloc: Allocator,
    terminal: terminal.Terminal,
    stream: terminal.Stream(*Handler),
    handler: Handler,
    default_fg: terminal.color.RGB,
    default_bg: terminal.color.RGB,
    viewport_top_y_screen: u32,
    has_viewport_top_y_screen: bool,
    cursor_style_code: u8,
    bell_pending: bool,
    grid_events: std.ArrayList(u8),
    on_alt_screen: bool,
    /// Tracked pin at the active-area top as of the last
    /// `ghostty_vt_terminal_take_scrolled_rows` call. ghostty keeps it valid
    /// across scrollback pruning, resize, and RIS (the reset repositions it),
    /// so the distance from it to the current active-area top yields a true
    /// monotonic per-feed scroll count even after the bounded scrollback ring
    /// saturates. `null` until the first call.
    capture_watermark: ?*terminal.Pin,

    fn init(alloc: Allocator, cols: u16, rows: u16, max_scrollback: usize) !*TerminalHandle {
        const handle = try alloc.create(TerminalHandle);
        errdefer alloc.destroy(handle);

        const t = try terminal.Terminal.init(alloc, .{
            .cols = cols,
            .rows = rows,
            .max_scrollback = max_scrollback,
        });
        errdefer {
            var tmp = t;
            tmp.deinit(alloc);
        }

        handle.* = .{
            .alloc = alloc,
            .terminal = t,
            .handler = .{ .terminal = undefined },
            .stream = undefined,
            .default_fg = .{ .r = 0xFF, .g = 0xFF, .b = 0xFF },
            .default_bg = .{ .r = 0x00, .g = 0x00, .b = 0x00 },
            .viewport_top_y_screen = 0,
            .has_viewport_top_y_screen = true,
            .cursor_style_code = 0,
            .bell_pending = false,
            .grid_events = std.ArrayList(u8).init(alloc),
            .on_alt_screen = false,
            .capture_watermark = null,
        };
        handle.handler.terminal = &handle.terminal;
        handle.stream = terminal.Stream(*Handler).init(&handle.handler);
        handle.stream.parser.osc_parser.alloc = alloc;
        return handle;
    }

    fn deinit(self: *TerminalHandle) void {
        self.stream.deinit();
        self.terminal.deinit(self.alloc);
        self.grid_events.deinit();
        self.alloc.destroy(self);
    }

    fn pushGridEvent(self: *TerminalHandle, tag: u8) void {
        self.grid_events.append(tag) catch return;
        self.grid_events.append(0) catch {
            // Roll back the tag byte so events stay 2-byte aligned.
            _ = self.grid_events.pop();
        };
    }
};

const Handler = struct {
    terminal: *terminal.Terminal,

    pub fn print(self: *Handler, c: u21) !void {
        try self.terminal.print(c);
    }

    pub fn backspace(self: *Handler) !void {
        self.terminal.backspace();
    }

    pub fn horizontalTab(self: *Handler, count: u16) !void {
        for (0..@as(usize, count)) |_| {
            try self.terminal.horizontalTab();
        }
    }

    pub fn linefeed(self: *Handler) !void {
        try self.terminal.linefeed();
    }

    pub fn carriageReturn(self: *Handler) !void {
        self.terminal.carriageReturn();
    }

    pub fn setAttribute(self: *Handler, attr: terminal.Attribute) !void {
        try self.terminal.setAttribute(attr);
    }

    pub fn invokeCharset(
        self: *Handler,
        active: terminal.CharsetActiveSlot,
        slot: terminal.CharsetSlot,
        single: bool,
    ) !void {
        self.terminal.invokeCharset(active, slot, single);
    }

    pub fn configureCharset(self: *Handler, slot: terminal.CharsetSlot, set: terminal.Charset) !void {
        self.terminal.configureCharset(slot, set);
    }

    pub fn handleColorOperation(
        self: *Handler,
        op: terminal.osc.color.Operation,
        requests: *const terminal.osc.color.List,
        terminator: terminal.osc.Terminator,
    ) !void {
        _ = op;
        _ = terminator;

        if (requests.count() == 0) return;

        var it = requests.constIterator(0);
        while (it.next()) |req| {
            switch (req.*) {
                .set => |set| switch (set.target) {
                    .palette => |i| {
                        self.terminal.color_palette.colors[i] = set.color;
                        self.terminal.color_palette.mask.set(i);
                        self.terminal.flags.dirty.palette = true;
                    },
                    else => {},
                },
                .reset => |target| switch (target) {
                    .palette => |i| {
                        self.terminal.color_palette.colors[i] = self.terminal.default_palette[i];
                        self.terminal.color_palette.mask.unset(i);
                        self.terminal.flags.dirty.palette = true;
                    },
                    else => {},
                },
                .reset_palette => {
                    const mask = &self.terminal.color_palette.mask;
                    var mask_iterator = mask.iterator(.{});
                    while (mask_iterator.next()) |idx| {
                        const i: usize = idx;
                        self.terminal.color_palette.colors[i] = self.terminal.default_palette[i];
                    }
                    self.terminal.color_palette.mask = .initEmpty();
                    self.terminal.flags.dirty.palette = true;
                },
                else => {},
            }
        }
    }

    pub fn setCursorLeft(self: *Handler, amount: u16) !void {
        self.terminal.cursorLeft(amount);
    }

    pub fn setCursorRight(self: *Handler, amount: u16) !void {
        self.terminal.cursorRight(amount);
    }

    pub fn setCursorDown(self: *Handler, amount: u16, carriage: bool) !void {
        self.terminal.cursorDown(amount);
        if (carriage) self.terminal.carriageReturn();
    }

    pub fn setCursorUp(self: *Handler, amount: u16, carriage: bool) !void {
        self.terminal.cursorUp(amount);
        if (carriage) self.terminal.carriageReturn();
    }

    pub fn setCursorCol(self: *Handler, col: u16) !void {
        self.terminal.setCursorPos(self.terminal.screen.cursor.y + 1, col);
    }

    pub fn setCursorRow(self: *Handler, row: u16) !void {
        self.terminal.setCursorPos(row, self.terminal.screen.cursor.x + 1);
    }

    pub fn setCursorPos(self: *Handler, row: u16, col: u16) !void {
        self.terminal.setCursorPos(row, col);
    }

    pub fn eraseDisplay(self: *Handler, mode: terminal.EraseDisplay, protected: bool) !void {
        self.terminal.eraseDisplay(mode, protected);
    }

    pub fn eraseLine(self: *Handler, mode: terminal.EraseLine, protected: bool) !void {
        self.terminal.eraseLine(mode, protected);
    }

    pub fn startHyperlink(self: *Handler, uri: []const u8, id: ?[]const u8) !void {
        try self.terminal.screen.startHyperlink(uri, id);
    }

    pub fn endHyperlink(self: *Handler) !void {
        self.terminal.screen.endHyperlink();
    }

    pub fn setCursorStyle(self: *Handler, style: terminal.CursorStyleReq) !void {
        const handle: *TerminalHandle = @fieldParentPtr("handler", self);
        const raw: u16 = @intFromEnum(style);
        handle.cursor_style_code = if (raw <= 255) @intCast(raw) else 0;
    }

    pub fn bell(self: *Handler) !void {
        const handle: *TerminalHandle = @fieldParentPtr("handler", self);
        handle.bell_pending = true;
    }

    pub fn insertLines(self: *Handler, count: u16) !void {
        self.terminal.insertLines(@intCast(count));
    }

    pub fn deleteLines(self: *Handler, count: u16) !void {
        self.terminal.deleteLines(@intCast(count));
    }

    pub fn insertBlanks(self: *Handler, count: u16) !void {
        self.terminal.insertBlanks(@intCast(count));
    }

    pub fn deleteChars(self: *Handler, count: u16) !void {
        self.terminal.deleteChars(@intCast(count));
    }

    pub fn eraseChars(self: *Handler, count: u16) !void {
        self.terminal.eraseChars(@intCast(count));
    }

    pub fn scrollUp(self: *Handler, count: u16) !void {
        self.terminal.scrollUp(@intCast(count));
    }

    pub fn scrollDown(self: *Handler, count: u16) !void {
        self.terminal.scrollDown(@intCast(count));
    }

    pub fn index(self: *Handler) !void {
        try self.terminal.index();
    }

    pub fn reverseIndex(self: *Handler) !void {
        self.terminal.reverseIndex();
    }

    pub fn nextLine(self: *Handler) !void {
        self.terminal.carriageReturn();
        try self.terminal.index();
    }

    pub fn setTopAndBottomMargin(self: *Handler, top: u16, bottom: u16) !void {
        self.terminal.setTopAndBottomMargin(@intCast(top), @intCast(bottom));
    }

    pub fn saveCursor(self: *Handler) !void {
        self.terminal.saveCursor();
    }

    pub fn restoreCursor(self: *Handler) !void {
        try self.terminal.restoreCursor();
    }

    pub fn tabSet(self: *Handler) !void {
        self.terminal.tabSet();
    }

    pub fn tabClear(self: *Handler, cmd: csi.TabClear) !void {
        self.terminal.tabClear(cmd);
    }

    pub fn tabReset(self: *Handler) !void {
        self.terminal.tabReset();
    }

    pub fn horizontalTabBack(self: *Handler, count: u16) !void {
        for (0..@as(usize, count)) |_| {
            try self.terminal.horizontalTabBack();
        }
    }

    pub fn printRepeat(self: *Handler, count: usize) !void {
        try self.terminal.printRepeat(count);
    }

    pub fn decaln(self: *Handler) !void {
        try self.terminal.decaln();
    }

    pub fn fullReset(self: *Handler) !void {
        const handle: *TerminalHandle = @fieldParentPtr("handler", self);
        self.terminal.fullReset();
        // RIS (ESC c) clears scrollback and resets every screen-affecting
        // mode, including alt-screen → primary; record both transitions.
        // Assumes ghostty's terminal.fullReset() does not internally call
        // setMode; otherwise the implicit ALT_SCREEN_EXIT below would
        // double-fire.
        if (handle.on_alt_screen) {
            handle.pushGridEvent(GRID_EVENT_ALT_SCREEN_EXIT);
            handle.on_alt_screen = false;
        }
        handle.pushGridEvent(GRID_EVENT_RIS);
    }

    pub fn setMode(self: *Handler, mode: terminal.Mode, enabled: bool) !void {
        const prev = self.terminal.modes.get(mode);
        self.terminal.modes.set(mode, enabled);

        if (prev != enabled) {
            switch (mode) {
                .reverse_colors => self.terminal.flags.dirty.reverse_colors = true,
                else => {},
            }
        }

        const handle: *TerminalHandle = @fieldParentPtr("handler", self);
        switch (mode) {
            .alt_screen_legacy => {
                self.terminal.switchScreenMode(.@"47", enabled);
                // `on_alt_screen` is shared across all three alt-screen mode variants
                // (47, 1047, 1049); mixing them therefore suppresses redundant transitions.
                if (enabled != handle.on_alt_screen) {
                    handle.pushGridEvent(if (enabled)
                        GRID_EVENT_ALT_SCREEN_ENTER
                    else
                        GRID_EVENT_ALT_SCREEN_EXIT);
                    handle.on_alt_screen = enabled;
                }
            },
            .alt_screen => {
                self.terminal.switchScreenMode(.@"1047", enabled);
                if (enabled != handle.on_alt_screen) {
                    handle.pushGridEvent(if (enabled)
                        GRID_EVENT_ALT_SCREEN_ENTER
                    else
                        GRID_EVENT_ALT_SCREEN_EXIT);
                    handle.on_alt_screen = enabled;
                }
            },
            .alt_screen_save_cursor_clear_enter => {
                self.terminal.switchScreenMode(.@"1049", enabled);
                if (enabled != handle.on_alt_screen) {
                    handle.pushGridEvent(if (enabled)
                        GRID_EVENT_ALT_SCREEN_ENTER
                    else
                        GRID_EVENT_ALT_SCREEN_EXIT);
                    handle.on_alt_screen = enabled;
                }
            },
            else => {},
        }
    }
};

export fn ghostty_vt_terminal_new(cols: u16, rows: u16, max_scrollback: usize) callconv(.C) ?*anyopaque {
    const alloc = std.heap.c_allocator;
    const handle = TerminalHandle.init(alloc, cols, rows, max_scrollback) catch return null;
    return @ptrCast(handle);
}

export fn ghostty_vt_terminal_free(terminal_ptr: ?*anyopaque) callconv(.C) void {
    if (terminal_ptr == null) return;
    const handle: *TerminalHandle = @ptrCast(@alignCast(terminal_ptr.?));
    handle.deinit();
}

export fn ghostty_vt_terminal_set_default_colors(
    terminal_ptr: ?*anyopaque,
    fg_r: u8,
    fg_g: u8,
    fg_b: u8,
    bg_r: u8,
    bg_g: u8,
    bg_b: u8,
) callconv(.C) void {
    if (terminal_ptr == null) return;
    const handle: *TerminalHandle = @ptrCast(@alignCast(terminal_ptr.?));
    handle.default_fg = .{ .r = fg_r, .g = fg_g, .b = fg_b };
    handle.default_bg = .{ .r = bg_r, .g = bg_g, .b = bg_b };
}

export fn ghostty_vt_terminal_feed(
    terminal_ptr: ?*anyopaque,
    bytes: [*]const u8,
    len: usize,
) callconv(.C) c_int {
    if (terminal_ptr == null) return 1;
    const handle: *TerminalHandle = @ptrCast(@alignCast(terminal_ptr.?));

    var i: usize = 0;
    while (i < len) : (i += 1) {
        handle.stream.next(bytes[i]) catch return 2;
    }

    return 0;
}

export fn ghostty_vt_terminal_resize(
    terminal_ptr: ?*anyopaque,
    cols: u16,
    rows: u16,
) callconv(.C) c_int {
    if (terminal_ptr == null) return 1;
    const handle: *TerminalHandle = @ptrCast(@alignCast(terminal_ptr.?));

    handle.terminal.resize(
        handle.alloc,
        @as(terminal.size.CellCountInt, @intCast(cols)),
        @as(terminal.size.CellCountInt, @intCast(rows)),
    ) catch return 2;
    return 0;
}

export fn ghostty_vt_terminal_scroll_viewport(
    terminal_ptr: ?*anyopaque,
    delta_lines: i32,
) callconv(.C) c_int {
    if (terminal_ptr == null) return 1;
    const handle: *TerminalHandle = @ptrCast(@alignCast(terminal_ptr.?));

    handle.terminal.scrollViewport(.{ .delta = @as(isize, delta_lines) }) catch return 2;
    return 0;
}

export fn ghostty_vt_terminal_scroll_viewport_top(terminal_ptr: ?*anyopaque) callconv(.C) c_int {
    if (terminal_ptr == null) return 1;
    const handle: *TerminalHandle = @ptrCast(@alignCast(terminal_ptr.?));

    handle.terminal.scrollViewport(.top) catch return 2;
    return 0;
}

export fn ghostty_vt_terminal_scroll_viewport_bottom(terminal_ptr: ?*anyopaque) callconv(.C) c_int {
    if (terminal_ptr == null) return 1;
    const handle: *TerminalHandle = @ptrCast(@alignCast(terminal_ptr.?));

    handle.terminal.scrollViewport(.bottom) catch return 2;
    return 0;
}

export fn ghostty_vt_terminal_cursor_position(
    terminal_ptr: ?*anyopaque,
    col_out: ?*u16,
    row_out: ?*u16,
) callconv(.C) bool {
    if (terminal_ptr == null) return false;
    if (col_out == null or row_out == null) return false;
    const handle: *TerminalHandle = @ptrCast(@alignCast(terminal_ptr.?));

    var x = handle.terminal.screen.cursor.x;
    // Snap off a wide-char spacer_tail to its head cell (xterm / iTerm2
    // behaviour: the reported cursor column is always the leftmost cell of
    // the wide character, never the phantom right half).
    if (x > 0 and handle.terminal.screen.cursor.page_cell.wide == .spacer_tail) {
        x -= 1;
    }
    col_out.?.* = @intCast(x + 1);
    row_out.?.* = @intCast(handle.terminal.screen.cursor.y + 1);
    return true;
}

export fn ghostty_vt_terminal_dump_viewport(terminal_ptr: ?*anyopaque) callconv(.C) ghostty_vt_bytes_t {
    if (terminal_ptr == null) return .{ .ptr = null, .len = 0 };
    const handle: *TerminalHandle = @ptrCast(@alignCast(terminal_ptr.?));

    const alloc = std.heap.c_allocator;
    const slice = handle.terminal.screen.dumpStringAlloc(alloc, .{ .viewport = .{} }) catch {
        return .{ .ptr = null, .len = 0 };
    };

    return .{ .ptr = slice.ptr, .len = slice.len };
}

export fn ghostty_vt_terminal_dump_viewport_row(
    terminal_ptr: ?*anyopaque,
    row: u16,
) callconv(.C) ghostty_vt_bytes_t {
    if (terminal_ptr == null) return .{ .ptr = null, .len = 0 };
    const handle: *TerminalHandle = @ptrCast(@alignCast(terminal_ptr.?));

    const pt: terminal.point.Point = .{ .viewport = .{ .x = 0, .y = row } };
    const pin = handle.terminal.screen.pages.pin(pt) orelse return .{ .ptr = null, .len = 0 };

    const alloc = std.heap.c_allocator;
    var builder = std.ArrayList(u8).init(alloc);
    errdefer builder.deinit();

    handle.terminal.screen.pages.encodeUtf8(builder.writer(), .{
        .tl = pin,
        .br = pin,
        .unwrap = false,
    }) catch return .{ .ptr = null, .len = 0 };

    const slice = builder.toOwnedSlice() catch return .{ .ptr = null, .len = 0 };
    return .{ .ptr = slice.ptr, .len = slice.len };
}

/// Dump a single row from the `screen` point space (scrollback + active
/// area). `y` is 0-based and may exceed the viewport height; callers
/// should clamp to `ghostty_vt_terminal_total_rows` first.
export fn ghostty_vt_terminal_dump_screen_row(
    terminal_ptr: ?*anyopaque,
    y: u32,
) callconv(.C) ghostty_vt_bytes_t {
    if (terminal_ptr == null) return .{ .ptr = null, .len = 0 };
    const handle: *TerminalHandle = @ptrCast(@alignCast(terminal_ptr.?));

    const pt: terminal.point.Point = .{ .screen = .{ .x = 0, .y = y } };
    const pin = handle.terminal.screen.pages.pin(pt) orelse return .{ .ptr = null, .len = 0 };

    const alloc = std.heap.c_allocator;
    var builder = std.ArrayList(u8).init(alloc);
    errdefer builder.deinit();

    handle.terminal.screen.pages.encodeUtf8(builder.writer(), .{
        .tl = pin,
        .br = pin,
        .unwrap = false,
    }) catch return .{ .ptr = null, .len = 0 };

    const slice = builder.toOwnedSlice() catch return .{ .ptr = null, .len = 0 };
    return .{ .ptr = slice.ptr, .len = slice.len };
}

const CellStyle = extern struct {
    fg_r: u8,
    fg_g: u8,
    fg_b: u8,
    bg_r: u8,
    bg_g: u8,
    bg_b: u8,
    flags: u8,
    reserved: u8,
};

export fn ghostty_vt_terminal_dump_viewport_row_cell_styles(
    terminal_ptr: ?*anyopaque,
    row: u16,
) callconv(.C) ghostty_vt_bytes_t {
    if (terminal_ptr == null) return .{ .ptr = null, .len = 0 };
    const handle: *TerminalHandle = @ptrCast(@alignCast(terminal_ptr.?));
    return dumpViewportRowCellStylesInner(handle, row) catch return .{ .ptr = null, .len = 0 };
}

fn dumpViewportRowCellStylesInner(
    handle: *TerminalHandle,
    row: u16,
) !ghostty_vt_bytes_t {
    const pt: terminal.point.Point = .{ .viewport = .{ .x = 0, .y = row } };
    const pin = handle.terminal.screen.pages.pin(pt) orelse return .{ .ptr = null, .len = 0 };
    const cells = pin.cells(.all);

    const default_fg: terminal.color.RGB = handle.default_fg;
    const default_bg: terminal.color.RGB = handle.default_bg;
    const palette: *const terminal.color.Palette = &handle.terminal.color_palette.colors;

    const alloc = std.heap.c_allocator;
    var out = std.ArrayList(u8).init(alloc);
    errdefer out.deinit();

    try out.ensureTotalCapacity(cells.len * @sizeOf(CellStyle));

    for (cells) |*cell| {
        const s = pin.style(cell);

        var fg = s.fg(.{ .default = default_fg, .palette = palette, .bold = null });
        var bg = s.bg(cell, palette) orelse default_bg;

        var flags: u8 = 0;
        if (s.flags.inverse) flags |= 0x01;
        if (s.flags.bold) flags |= 0x02;
        if (s.flags.italic) flags |= 0x04;
        if (s.flags.underline != .none) flags |= 0x08;
        if (s.flags.faint) flags |= 0x10;
        if (s.flags.invisible) flags |= 0x20;
        if (s.flags.strikethrough) flags |= 0x40;

        if (s.flags.inverse) {
            const tmp = fg;
            fg = bg;
            bg = tmp;
        }
        if (s.flags.invisible) {
            fg = bg;
        }

        const rec = CellStyle{
            .fg_r = fg.r,
            .fg_g = fg.g,
            .fg_b = fg.b,
            .bg_r = bg.r,
            .bg_g = bg.g,
            .bg_b = bg.b,
            .flags = flags,
            .reserved = 0,
        };
        try out.appendSlice(std.mem.asBytes(&rec));
    }

    const slice = try out.toOwnedSlice();
    return .{ .ptr = slice.ptr, .len = slice.len };
}

const StyleRun = extern struct {
    start_col: u16,
    end_col: u16,
    fg_r: u8,
    fg_g: u8,
    fg_b: u8,
    bg_r: u8,
    bg_g: u8,
    bg_b: u8,
    flags: u8,
    reserved: u8,
};

fn resolvedStyle(
    default_fg: terminal.color.RGB,
    default_bg: terminal.color.RGB,
    palette: *const terminal.color.Palette,
    s: anytype,
) struct {
    fg: terminal.color.RGB,
    bg: terminal.color.RGB,
    flags: u8,
} {
    var flags: u8 = 0;
    if (s.flags.inverse) flags |= 0x01;
    if (s.flags.bold) flags |= 0x02;
    if (s.flags.italic) flags |= 0x04;
    if (s.flags.underline != .none) flags |= 0x08;
    if (s.flags.faint) flags |= 0x10;
    if (s.flags.invisible) flags |= 0x20;
    if (s.flags.strikethrough) flags |= 0x40;

    const fg = s.fg(.{ .default = default_fg, .palette = palette, .bold = null });
    return .{ .fg = fg, .bg = default_bg, .flags = flags };
}

export fn ghostty_vt_terminal_dump_viewport_row_style_runs(
    terminal_ptr: ?*anyopaque,
    row: u16,
) callconv(.C) ghostty_vt_bytes_t {
    if (terminal_ptr == null) return .{ .ptr = null, .len = 0 };
    const handle: *TerminalHandle = @ptrCast(@alignCast(terminal_ptr.?));
    return dumpRowStyleRunsImpl(handle, .{ .viewport = .{ .x = 0, .y = row } });
}

/// Screen-coordinate variant of `ghostty_vt_terminal_dump_viewport_row_style_runs`.
/// `y` ranges over `[0, total_rows())`, covering scrollback and the
/// active viewport. Used by `LineBuffer` to capture style runs for rows
/// about to scroll out of the viewport.
export fn ghostty_vt_terminal_dump_screen_row_style_runs(
    terminal_ptr: ?*anyopaque,
    y: u32,
) callconv(.C) ghostty_vt_bytes_t {
    if (terminal_ptr == null) return .{ .ptr = null, .len = 0 };
    const handle: *TerminalHandle = @ptrCast(@alignCast(terminal_ptr.?));
    return dumpRowStyleRunsImpl(handle, .{ .screen = .{ .x = 0, .y = y } });
}

fn dumpRowStyleRunsImpl(
    handle: *TerminalHandle,
    pt: terminal.point.Point,
) ghostty_vt_bytes_t {
    return dumpRowStyleRunsInner(handle, pt) catch return .{ .ptr = null, .len = 0 };
}

fn dumpRowStyleRunsInner(
    handle: *TerminalHandle,
    pt: terminal.point.Point,
) !ghostty_vt_bytes_t {
    const pin = handle.terminal.screen.pages.pin(pt) orelse return .{ .ptr = null, .len = 0 };
    const cells = pin.cells(.all);

    const default_fg: terminal.color.RGB = handle.default_fg;
    const default_bg: terminal.color.RGB = handle.default_bg;
    const palette: *const terminal.color.Palette = &handle.terminal.color_palette.colors;

    const alloc = std.heap.c_allocator;
    var out = std.ArrayList(u8).init(alloc);
    errdefer out.deinit();

    if (cells.len == 0) {
        const slice = try out.toOwnedSlice();
        return .{ .ptr = slice.ptr, .len = slice.len };
    }

    var current_style_id = cells[0].style_id;
    var current_style = pin.style(&cells[0]);
    const defaults = resolvedStyle(default_fg, default_bg, palette, current_style);

    var current_flags = defaults.flags;
    var current_base_fg = defaults.fg;
    var current_inverse = current_style.flags.inverse;
    var current_invisible = current_style.flags.invisible;

    var current_bg = current_style.bg(&cells[0], palette) orelse default_bg;
    var current_fg = current_base_fg;
    if (current_inverse) {
        const tmp = current_fg;
        current_fg = current_bg;
        current_bg = tmp;
    }
    if (current_invisible) {
        current_fg = current_bg;
    }

    var current_resolved = .{ .fg = current_fg, .bg = current_bg, .flags = current_flags };
    // `start_col` / `end_col` are 1-indexed inclusive (see
    // `text_metrics::byte_index_for_column_in_line` — col 1 == first cell).
    // The loop starts at `col_idx = 1` (0-indexed) so column 0's style is
    // captured by the initial `current_resolved`, and a run break at
    // `col_idx` emits `end_col = col_idx` (= 1-indexed `col_idx`), with
    // the next run starting at `run_start = col_idx + 1`.
    var run_start: u16 = 1;

    var col_idx: usize = 1;
    while (col_idx < cells.len) : (col_idx += 1) {
        const cell = &cells[col_idx];
        if (cell.style_id != current_style_id) {
            const end_col: u16 = @intCast(col_idx);
            const rec = StyleRun{
                .start_col = run_start,
                .end_col = end_col,
                .fg_r = current_resolved.fg.r,
                .fg_g = current_resolved.fg.g,
                .fg_b = current_resolved.fg.b,
                .bg_r = current_resolved.bg.r,
                .bg_g = current_resolved.bg.g,
                .bg_b = current_resolved.bg.b,
                .flags = current_resolved.flags,
                .reserved = 0,
            };
            try out.appendSlice(std.mem.asBytes(&rec));

            current_style_id = cell.style_id;
            current_style = pin.style(cell);
            const resolved = resolvedStyle(default_fg, default_bg, palette, current_style);
            current_flags = resolved.flags;
            current_base_fg = resolved.fg;
            current_inverse = current_style.flags.inverse;
            current_invisible = current_style.flags.invisible;

            run_start = @intCast(col_idx + 1);

            const bg_cell = current_style.bg(cell, palette) orelse default_bg;
            var fg_cell = current_base_fg;
            var bg = bg_cell;
            if (current_inverse) {
                const tmp = fg_cell;
                fg_cell = bg;
                bg = tmp;
            }
            if (current_invisible) {
                fg_cell = bg;
            }

            current_resolved = .{ .fg = fg_cell, .bg = bg, .flags = current_flags };
            continue;
        }

        const bg_cell = current_style.bg(cell, palette) orelse default_bg;
        var fg_cell = current_base_fg;
        var bg = bg_cell;
        if (current_inverse) {
            const tmp = fg_cell;
            fg_cell = bg;
            bg = tmp;
        }
        if (current_invisible) {
            fg_cell = bg;
        }

        const same = fg_cell.r == current_resolved.fg.r and fg_cell.g == current_resolved.fg.g and fg_cell.b == current_resolved.fg.b and
            bg.r == current_resolved.bg.r and bg.g == current_resolved.bg.g and bg.b == current_resolved.bg.b and
            current_flags == current_resolved.flags;
        if (same) continue;

        const end_col: u16 = @intCast(col_idx);
        const rec = StyleRun{
            .start_col = run_start,
            .end_col = end_col,
            .fg_r = current_resolved.fg.r,
            .fg_g = current_resolved.fg.g,
            .fg_b = current_resolved.fg.b,
            .bg_r = current_resolved.bg.r,
            .bg_g = current_resolved.bg.g,
            .bg_b = current_resolved.bg.b,
            .flags = current_resolved.flags,
            .reserved = 0,
        };
        try out.appendSlice(std.mem.asBytes(&rec));

        run_start = @intCast(col_idx + 1);
        current_resolved = .{ .fg = fg_cell, .bg = bg, .flags = current_flags };
    }

    const last = StyleRun{
        .start_col = run_start,
        .end_col = @intCast(cells.len),
        .fg_r = current_resolved.fg.r,
        .fg_g = current_resolved.fg.g,
        .fg_b = current_resolved.fg.b,
        .bg_r = current_resolved.bg.r,
        .bg_g = current_resolved.bg.g,
        .bg_b = current_resolved.bg.b,
        .flags = current_resolved.flags,
        .reserved = 0,
    };
    try out.appendSlice(std.mem.asBytes(&last));

    const slice = try out.toOwnedSlice();
    return .{ .ptr = slice.ptr, .len = slice.len };
}

/// Dump per-cell OSC 8 hyperlink IDs for a row in screen coordinates.
/// `y` ranges over `[0, total_rows())`. Returns `cells * 2` bytes — one
/// little-endian `u16` per cell, with `0` meaning "no hyperlink". Used by
/// `LineBuffer::attach_url_ids_to_tail` so OSC 8 link IDs survive a row
/// scrolling out of the viewport.
export fn ghostty_vt_terminal_dump_screen_row_url_ids(
    terminal_ptr: ?*anyopaque,
    y: u32,
) callconv(.C) ghostty_vt_bytes_t {
    if (terminal_ptr == null) return .{ .ptr = null, .len = 0 };
    const handle: *TerminalHandle = @ptrCast(@alignCast(terminal_ptr.?));
    return dumpScreenRowUrlIdsInner(handle, y) catch return .{ .ptr = null, .len = 0 };
}

fn dumpScreenRowUrlIdsInner(
    handle: *TerminalHandle,
    y: u32,
) !ghostty_vt_bytes_t {
    const pt: terminal.point.Point = .{ .screen = .{ .x = 0, .y = y } };
    const pin = handle.terminal.screen.pages.pin(pt) orelse return .{ .ptr = null, .len = 0 };
    const cells = pin.cells(.all);

    const alloc = std.heap.c_allocator;

    // Drive the id emission off the SAME `encodeUtf8` traversal that
    // `dump_screen_row` produces text from: `cell_map` records the source
    // cell of every emitted codepoint (skipping spacers, trimming trailing
    // blanks, expanding graphemes). Emitting one id per cell-map entry
    // therefore yields exactly one id per text char, aligned 1:1 — instead
    // of one per physical cell, which over-counted wide-char spacers and
    // trailing blanks and shifted ids onto the wrong cells.
    var cell_map = terminal.Page.CellMap.init(alloc);
    defer cell_map.deinit();
    var text = std.ArrayList(u8).init(alloc);
    defer text.deinit();
    try handle.terminal.screen.pages.encodeUtf8(text.writer(), .{
        .tl = pin,
        .br = pin,
        .unwrap = false,
        .cell_map = &cell_map,
    });

    var out = std.ArrayList(u8).init(alloc);
    errdefer out.deinit();
    try out.ensureTotalCapacity(cell_map.items.len * @sizeOf(u16));

    for (cell_map.items) |entry| {
        var id: u16 = 0;
        // Single-row dump (tl == br): every entry sits on the pin's row,
        // so the cell is `cells[entry.x]`. Guard the index defensively.
        if (entry.x < cells.len) {
            const cell = &cells[entry.x];
            if (cell.hyperlink) {
                if (pin.node.data.lookupHyperlink(cell)) |link_id| {
                    id = @intCast(link_id);
                }
            }
        }
        try out.appendSlice(std.mem.asBytes(&id));
    }

    const slice = try out.toOwnedSlice();
    return .{ .ptr = slice.ptr, .len = slice.len };
}

export fn ghostty_vt_terminal_take_dirty_viewport_rows(
    terminal_ptr: ?*anyopaque,
    rows: u16,
) callconv(.C) ghostty_vt_bytes_t {
    if (terminal_ptr == null or rows == 0) return .{ .ptr = null, .len = 0 };
    const handle: *TerminalHandle = @ptrCast(@alignCast(terminal_ptr.?));

    const alloc = std.heap.c_allocator;

    var out = std.ArrayList(u8).init(alloc);
    errdefer out.deinit();

    const dirty = handle.terminal.flags.dirty;
    const force_full_redraw = dirty.clear or dirty.palette or dirty.reverse_colors or dirty.preedit;
    if (force_full_redraw) {
        handle.terminal.flags.dirty.clear = false;
        handle.terminal.flags.dirty.palette = false;
        handle.terminal.flags.dirty.reverse_colors = false;
        handle.terminal.flags.dirty.preedit = false;
    }

    var y: u32 = 0;
    while (y < rows) : (y += 1) {
        const pt: terminal.point.Point = .{ .viewport = .{ .x = 0, .y = y } };
        const pin = handle.terminal.screen.pages.pin(pt) orelse continue;
        if (!force_full_redraw and !pin.isDirty()) continue;

        const v: u16 = @intCast(y);
        out.append(@intCast(v & 0xFF)) catch return .{ .ptr = null, .len = 0 };
        out.append(@intCast((v >> 8) & 0xFF)) catch return .{ .ptr = null, .len = 0 };

        var set = pin.node.data.dirtyBitSet();
        set.unset(@intCast(pin.y));
    }

    const slice = out.toOwnedSlice() catch return .{ .ptr = null, .len = 0 };
    return .{ .ptr = slice.ptr, .len = slice.len };
}

/// Drain pending grid events accumulated by mode/reset handlers. Each
/// event is encoded as a 2-byte record `[tag, 0]` where `tag` is one of
/// `GRID_EVENT_ALT_SCREEN_ENTER`, `GRID_EVENT_ALT_SCREEN_EXIT`, or
/// `GRID_EVENT_RIS`. Returns an empty buffer when no events are pending.
export fn ghostty_vt_terminal_take_grid_events(
    terminal_ptr: ?*anyopaque,
) callconv(.C) ghostty_vt_bytes_t {
    if (terminal_ptr == null) return .{ .ptr = null, .len = 0 };
    const handle: *TerminalHandle = @ptrCast(@alignCast(terminal_ptr.?));

    if (handle.grid_events.items.len == 0) {
        return .{ .ptr = null, .len = 0 };
    }

    const slice = handle.grid_events.toOwnedSlice() catch return .{ .ptr = null, .len = 0 };
    return .{ .ptr = slice.ptr, .len = slice.len };
}

fn pinScreenRow(pin: terminal.Pin) u32 {
    var y: u32 = @intCast(pin.y);
    var node_ = pin.node;
    while (node_.prev) |node| {
        y += @intCast(node.data.size.rows);
        node_ = node;
    }
    return y;
}

export fn ghostty_vt_terminal_cursor_style(terminal_ptr: ?*anyopaque) callconv(.C) u8 {
    if (terminal_ptr == null) return 0;
    const handle: *TerminalHandle = @ptrCast(@alignCast(terminal_ptr.?));
    return handle.cursor_style_code;
}

export fn ghostty_vt_terminal_cursor_visible(terminal_ptr: ?*anyopaque) callconv(.C) bool {
    if (terminal_ptr == null) return true;
    const handle: *TerminalHandle = @ptrCast(@alignCast(terminal_ptr.?));
    return handle.terminal.modes.get(.cursor_visible);
}

export fn ghostty_vt_terminal_take_bell(terminal_ptr: ?*anyopaque) callconv(.C) bool {
    if (terminal_ptr == null) return false;
    const handle: *TerminalHandle = @ptrCast(@alignCast(terminal_ptr.?));
    const pending = handle.bell_pending;
    handle.bell_pending = false;
    return pending;
}

export fn ghostty_vt_terminal_total_rows(terminal_ptr: ?*anyopaque) callconv(.C) u32 {
    if (terminal_ptr == null) return 0;
    const handle: *TerminalHandle = @ptrCast(@alignCast(terminal_ptr.?));
    var rows: u32 = 0;
    var node_ = handle.terminal.screen.pages.pages.first;
    while (node_) |node| {
        rows += @intCast(node.data.size.rows);
        node_ = node.next;
    }
    return rows;
}

export fn ghostty_vt_terminal_viewport_row_offset(terminal_ptr: ?*anyopaque) callconv(.C) u32 {
    if (terminal_ptr == null) return 0;
    const handle: *TerminalHandle = @ptrCast(@alignCast(terminal_ptr.?));
    const tl = handle.terminal.screen.pages.getTopLeft(.viewport);
    return pinScreenRow(tl);
}

/// Number of rows that scrolled off the active area into scrollback history
/// since the previous call (0 on the first call after a reset window).
///
/// Implemented with a tracked pin ("capture watermark") parked at the
/// active-area top. Because ghostty repositions tracked pins when it prunes
/// scrollback, the row distance between the watermark and the current
/// active-area top stays correct even after the bounded scrollback ring
/// saturates — unlike `viewport_row_offset`, which plateaus. This is the
/// monotonic per-feed scroll count the LineBuffer capture path needs so it
/// never freezes at the ring size.
///
/// The watermark is repositioned in place each call. ghostty preserves it
/// across RIS (`Screen.reset` keeps tracked pins) and it lives in the primary
/// screen's page list; capture is skipped while on the alternate screen, so
/// it is never read against the wrong page list.
export fn ghostty_vt_terminal_take_scrolled_rows(terminal_ptr: ?*anyopaque) callconv(.C) u32 {
    if (terminal_ptr == null) return 0;
    const handle: *TerminalHandle = @ptrCast(@alignCast(terminal_ptr.?));
    const pages = &handle.terminal.screen.pages;
    const active_top = pages.getTopLeft(.active);
    const active_row = pinScreenRow(active_top);
    if (handle.capture_watermark) |wm| {
        // active_row >= watermark row in steady state; saturating-sub guards a
        // watermark that ghostty pulled back to row 0 on prune/RIS.
        const n = active_row -| pinScreenRow(wm.*);
        // Reposition the tracked pin in place; its pointer remains the key in
        // the tracked-pin set, so no re-tracking is required.
        wm.* = active_top;
        return n;
    }
    // First call: every row currently above the active area is uncaptured.
    handle.capture_watermark = pages.trackPin(active_top) catch return active_row;
    return active_row;
}

/// Same scroll-delta computation as `take_scrolled_rows` but
/// non-destructive: the watermark is not repositioned. Lets a caller
/// reason about "rows above the active area that haven't been captured
/// yet" without consuming the delta (e.g. computing an OSC 133 mark's
/// absolute Y mid-feed without disturbing the capture cursor that the
/// end-of-feed loop relies on).
export fn ghostty_vt_terminal_peek_scrolled_rows(terminal_ptr: ?*anyopaque) callconv(.C) u32 {
    if (terminal_ptr == null) return 0;
    const handle: *TerminalHandle = @ptrCast(@alignCast(terminal_ptr.?));
    const pages = &handle.terminal.screen.pages;
    const active_top = pages.getTopLeft(.active);
    const active_row = pinScreenRow(active_top);
    if (handle.capture_watermark) |wm| {
        return active_row -| pinScreenRow(wm.*);
    }
    // No watermark yet: mirrors the take-side first-call result.
    return active_row;
}

export fn ghostty_vt_terminal_take_viewport_scroll_delta(
    terminal_ptr: ?*anyopaque,
) callconv(.C) i32 {
    if (terminal_ptr == null) return 0;
    const handle: *TerminalHandle = @ptrCast(@alignCast(terminal_ptr.?));

    const tl = handle.terminal.screen.pages.getTopLeft(.viewport);
    const current: u32 = pinScreenRow(tl);

    if (!handle.has_viewport_top_y_screen) {
        handle.viewport_top_y_screen = current;
        handle.has_viewport_top_y_screen = true;
        return 0;
    }

    const prev: u32 = handle.viewport_top_y_screen;
    handle.viewport_top_y_screen = current;

    const delta64: i64 = @as(i64, @intCast(current)) - @as(i64, @intCast(prev));
    if (delta64 > std.math.maxInt(i32)) return std.math.maxInt(i32);
    if (delta64 < std.math.minInt(i32)) return std.math.minInt(i32);
    return @intCast(delta64);
}

export fn ghostty_vt_terminal_hyperlink_at(
    terminal_ptr: ?*anyopaque,
    col: u16,
    row: u16,
) callconv(.C) ghostty_vt_bytes_t {
    if (terminal_ptr == null or col == 0 or row == 0) return .{ .ptr = null, .len = 0 };
    const handle: *TerminalHandle = @ptrCast(@alignCast(terminal_ptr.?));

    const x: terminal.size.CellCountInt = @intCast(col - 1);
    const y: u32 = @intCast(row - 1);
    const pt: terminal.point.Point = .{ .viewport = .{ .x = x, .y = y } };
    const pin = handle.terminal.screen.pages.pin(pt) orelse return .{ .ptr = null, .len = 0 };
    const rac = pin.rowAndCell();
    if (!rac.cell.hyperlink) return .{ .ptr = null, .len = 0 };

    const id = pin.node.data.lookupHyperlink(rac.cell) orelse return .{ .ptr = null, .len = 0 };
    const entry = pin.node.data.hyperlink_set.get(pin.node.data.memory, id).*;
    const uri = entry.uri.offset.ptr(pin.node.data.memory)[0..entry.uri.len];

    const alloc = std.heap.c_allocator;
    const duped = alloc.dupe(u8, uri) catch return .{ .ptr = null, .len = 0 };
    return .{ .ptr = duped.ptr, .len = duped.len };
}

export fn ghostty_vt_encode_key_named(
    name_ptr: ?[*]const u8,
    name_len: usize,
    modifiers: u16,
    flags: u8,
) callconv(.C) ghostty_vt_bytes_t {
    if (name_ptr == null or name_len == 0) return .{ .ptr = null, .len = 0 };

    const name = name_ptr.?[0..name_len];

    const key_value: ghostty_input.Key = if (std.mem.eql(u8, name, "up"))
        .arrow_up
    else if (std.mem.eql(u8, name, "down"))
        .arrow_down
    else if (std.mem.eql(u8, name, "left"))
        .arrow_left
    else if (std.mem.eql(u8, name, "right"))
        .arrow_right
    else if (std.mem.eql(u8, name, "home"))
        .home
    else if (std.mem.eql(u8, name, "end"))
        .end
    else if (std.mem.eql(u8, name, "pageup") or std.mem.eql(u8, name, "page_up") or std.mem.eql(u8, name, "page-up"))
        .page_up
    else if (std.mem.eql(u8, name, "pagedown") or std.mem.eql(u8, name, "page_down") or std.mem.eql(u8, name, "page-down"))
        .page_down
    else if (std.mem.eql(u8, name, "insert"))
        .insert
    else if (std.mem.eql(u8, name, "delete"))
        .delete
    else if (std.mem.eql(u8, name, "backspace"))
        .backspace
    else if (std.mem.eql(u8, name, "enter"))
        .enter
    else if (std.mem.eql(u8, name, "tab"))
        .tab
    else if (std.mem.eql(u8, name, "escape"))
        .escape
    else if (name.len >= 2 and name[0] == 'f')
        parse_function_key(name[1..]) orelse return .{ .ptr = null, .len = 0 }
    else
        return .{ .ptr = null, .len = 0 };

    var mods: ghostty_input.Mods = .{};
    if ((modifiers & 0x0001) != 0) mods.shift = true;
    if ((modifiers & 0x0002) != 0) mods.ctrl = true;
    if ((modifiers & 0x0004) != 0) mods.alt = true;
    if ((modifiers & 0x0008) != 0) mods.super = true;

    const event: ghostty_input.KeyEvent = .{
        .action = .press,
        .key = key_value,
        .mods = mods,
    };

    const enc: ghostty_input.KeyEncoder = .{
        .event = event,
        .alt_esc_prefix = true,
        .cursor_key_application = (flags & 0x01) != 0,
        .keypad_key_application = (flags & 0x02) != 0,
    };

    var buf: [128]u8 = undefined;
    const encoded = enc.encode(buf[0..]) catch return .{ .ptr = null, .len = 0 };
    if (encoded.len == 0) return .{ .ptr = null, .len = 0 };

    const alloc = std.heap.c_allocator;
    const duped = alloc.dupe(u8, encoded) catch return .{ .ptr = null, .len = 0 };
    return .{ .ptr = duped.ptr, .len = duped.len };
}

fn parse_function_key(digits: []const u8) ?ghostty_input.Key {
    if (digits.len == 1) {
        return switch (digits[0]) {
            '1' => .f1,
            '2' => .f2,
            '3' => .f3,
            '4' => .f4,
            '5' => .f5,
            '6' => .f6,
            '7' => .f7,
            '8' => .f8,
            '9' => .f9,
            else => null,
        };
    }

    if (digits.len == 2 and digits[0] == '1') {
        return switch (digits[1]) {
            '0' => .f10,
            '1' => .f11,
            '2' => .f12,
            else => null,
        };
    }

    return null;
}

const ghostty_vt_bytes_t = extern struct {
    ptr: ?[*]const u8,
    len: usize,
};

export fn ghostty_vt_bytes_free(bytes: ghostty_vt_bytes_t) callconv(.C) void {
    if (bytes.ptr == null or bytes.len == 0) return;
    std.heap.c_allocator.free(bytes.ptr.?[0..bytes.len]);
}

/// Returns 0 = Hard (no wrap), 1 = Soft (DECAWM wrap), -1 = invalid row.
/// Note: DWC handling (returning 2) is deferred; ghostty's per-row `wrap`
/// flag alone distinguishes hard vs soft. Callers default to Hard for unknown.
export fn ghostty_vt_terminal_row_wrap_kind(
    terminal_ptr: ?*anyopaque,
    y: u32,
) callconv(.C) c_int {
    if (terminal_ptr == null) return -1;
    const handle: *TerminalHandle = @ptrCast(@alignCast(terminal_ptr.?));
    const pt: terminal.point.Point = .{ .screen = .{ .x = 0, .y = y } };
    const pin = handle.terminal.screen.pages.pin(pt) orelse return -1;
    const row = pin.rowAndCell().row;
    if (row.wrap) {
        return 1;
    } else {
        return 0;
    }
}

// Ghostty's terminal stream uses this symbol as an optimization hook.
// Provide a portable scalar implementation so we don't need C++ SIMD deps.
export fn ghostty_simd_decode_utf8_until_control_seq(
    input: [*]const u8,
    count: usize,
    output: [*]u32,
    output_count: *usize,
) callconv(.C) usize {
    var i: usize = 0;
    var out_i: usize = 0;
    while (i < count) {
        if (input[i] == 0x1B) break;

        const b0 = input[i];
        var cp: u32 = 0xFFFD;
        var need: usize = 1;

        if (b0 < 0x80) {
            cp = b0;
            need = 1;
        } else if (b0 & 0xE0 == 0xC0) {
            need = 2;
            if (i + need > count) break;
            const b1 = input[i + 1];
            if (b1 & 0xC0 != 0x80) {
                cp = 0xFFFD;
                need = 1;
            } else {
                cp = ((@as(u32, b0 & 0x1F)) << 6) | (@as(u32, b1 & 0x3F));
            }
        } else if (b0 & 0xF0 == 0xE0) {
            need = 3;
            if (i + need > count) break;
            const b1 = input[i + 1];
            const b2 = input[i + 2];
            if (b1 & 0xC0 != 0x80 or b2 & 0xC0 != 0x80) {
                cp = 0xFFFD;
                need = 1;
            } else {
                cp = ((@as(u32, b0 & 0x0F)) << 12) |
                    ((@as(u32, b1 & 0x3F)) << 6) |
                    (@as(u32, b2 & 0x3F));
            }
        } else if (b0 & 0xF8 == 0xF0) {
            need = 4;
            if (i + need > count) break;
            const b1 = input[i + 1];
            const b2 = input[i + 2];
            const b3 = input[i + 3];
            if (b1 & 0xC0 != 0x80 or b2 & 0xC0 != 0x80 or b3 & 0xC0 != 0x80) {
                cp = 0xFFFD;
                need = 1;
            } else {
                cp = ((@as(u32, b0 & 0x07)) << 18) |
                    ((@as(u32, b1 & 0x3F)) << 12) |
                    ((@as(u32, b2 & 0x3F)) << 6) |
                    (@as(u32, b3 & 0x3F));
            }
        } else {
            cp = 0xFFFD;
            need = 1;
        }

        output[out_i] = cp;
        out_i += 1;
        i += need;
    }

    output_count.* = out_i;
    return i;
}
