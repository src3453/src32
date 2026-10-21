cmake_minimum_required(VERSION 3.20)
project(newlib_c LANGUAGES NONE)
include("../../tools/llvm-src32/cmake/SRC32.cmake")
set(CPT32_NEWLIB_SOURCE "" CACHE PATH "External Newlib source root")
if(NOT CPT32_NEWLIB_SOURCE)
    message(FATAL_ERROR "Set CPT32_NEWLIB_SOURCE to your separately obtained Newlib source root")
endif()
src32_add_executable(newlib_c SOURCES main.c OPTIMIZATION 1
    NEWLIB_SOURCE "${CPT32_NEWLIB_SOURCE}" NEWLIB_HEAP_SIZE 65536)
