cmake_minimum_required(VERSION 3.20)
project(sgu_input_scope LANGUAGES NONE)
add_subdirectory("${CMAKE_CURRENT_LIST_DIR}/../../sdk/cpt32" cpt32)
src32_add_executable(sgu_input_scope
    SOURCES main.c
    OPTIMIZATION 0
    ORIGIN 0
    STACK_TOP 0x00100000
    ENTRY main)
