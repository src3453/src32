cmake_minimum_required(VERSION 3.20)
# SRC32.cmake owns cross compilation; no host C compiler/linker is invoked.
project(src32_c_conformance LANGUAGES NONE)
include("${CMAKE_CURRENT_LIST_DIR}/../../cmake/SRC32.cmake")
set(SRC32_OPTIMIZATION 0 CACHE STRING "Clang optimization level: 0, 1 or 2")

src32_add_library(conformance_logic
    SOURCES operations.c integers.c wide.c
    INCLUDE_DIRECTORIES "${CMAKE_CURRENT_LIST_DIR}")
src32_add_executable(conformance
    SOURCES main.c
    LIBRARIES conformance_logic
    OPTIMIZATION "${SRC32_OPTIMIZATION}")
