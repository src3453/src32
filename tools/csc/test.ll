define dso_local @main() {
  %1 = alloca i32, align 4
  store i32 0, ptr %1, align 4
  ret i32 0, !dbg !15
}