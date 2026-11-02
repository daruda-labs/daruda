const std = @import("std");

pub fn build(b: *std.Build) void {
    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{});
    const upstream = b.dependency("ghostty", .{
        .target = target,
        .optimize = optimize,
        .@"app-runtime" = .none,
        .@"emit-exe" = false,
        .simd = false,
        // Daruda renders text cells and does not expose Kitty image placements.
        .@"vt-features" = "-kitty-graphics",
    });
    const lib = b.addLibrary(.{
        .name = "ghostty_vt",
        .root_module = b.createModule(.{
            .root_source_file = b.path("lib.zig"),
            .target = target,
            .optimize = optimize,
            .link_libc = true,
            .imports = &.{.{ .name = "ghostty-vt", .module = upstream.module("ghostty-vt") }},
        }),
        .linkage = .static,
    });
    b.installArtifact(lib);
    b.getInstallStep().dependOn(&b.addInstallHeaderFile(b.path("../include/ghostty_vt.h"), "ghostty_vt.h").step);
}
