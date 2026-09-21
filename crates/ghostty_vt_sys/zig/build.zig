const std = @import("std");

pub fn build(b: *std.Build) void {
    const optimize = b.standardOptimizeOption(.{});
    const target = b.standardTargetOptions(.{});
    const ghostty_source = b.option([]const u8, "ghostty-source", "Absolute Ghostty src directory") orelse
        @panic("-Dghostty-source is required");
    const sources = b.addWriteFiles();
    const ghostty = sources.addCopyDirectory(.{ .cwd_relative = ghostty_source }, "ghostty_src", .{});
    const wrapper = sources.addCopyFile(b.path("lib.zig"), "lib.zig");

    const ziglyph_host = b.dependency("ziglyph", .{
        .target = b.graph.host,
        .optimize = optimize,
    });

    const ziglyph_target = b.dependency("ziglyph", .{
        .target = target,
        .optimize = optimize,
    });

    const props_exe = b.addExecutable(.{
        .name = "props-unigen",
        .root_module = b.createModule(.{
            .root_source_file = ghostty.path(b, "unicode/props.zig"),
            .target = b.graph.host,
            .optimize = optimize,
        }),
    });
    props_exe.root_module.addImport("ziglyph", ziglyph_host.module("ziglyph"));

    const symbols_exe = b.addExecutable(.{
        .name = "symbols-unigen",
        .root_module = b.createModule(.{
            .root_source_file = ghostty.path(b, "unicode/symbols.zig"),
            .target = b.graph.host,
            .optimize = optimize,
        }),
    });
    symbols_exe.root_module.addImport("ziglyph", ziglyph_host.module("ziglyph"));

    const props_run = b.addRunArtifact(props_exe);
    const symbols_run = b.addRunArtifact(symbols_exe);
    const props_output = props_run.captureStdOut();
    const symbols_output = symbols_run.captureStdOut();

    const lib = b.addLibrary(.{
        .name = "ghostty_vt",
        .root_module = b.createModule(.{
            .root_source_file = wrapper,
            .target = target,
            .optimize = optimize,
        }),
        .linkage = .static,
    });
    lib.linkLibC();
    lib.root_module.addImport("ziglyph", ziglyph_target.module("ziglyph"));

    props_output.addStepDependencies(&lib.step);
    lib.root_module.addAnonymousImport("unicode_tables", .{
        .root_source_file = props_output,
    });
    symbols_output.addStepDependencies(&lib.step);
    lib.root_module.addAnonymousImport("symbols_tables", .{
        .root_source_file = symbols_output,
    });

    const include_step = b.addInstallHeaderFile(
        b.path("../include/ghostty_vt.h"),
        "ghostty_vt.h",
    );

    const lib_install = b.addInstallLibFile(lib.getEmittedBin(), "libghostty_vt.a");
    b.getInstallStep().dependOn(&include_step.step);
    b.getInstallStep().dependOn(&lib_install.step);
}
