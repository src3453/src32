cmake_minimum_required(VERSION 3.20)
project(tinybasic_c LANGUAGES NONE)

add_subdirectory("${CMAKE_CURRENT_LIST_DIR}/../../sdk/cpt32" cpt32)
src32_add_executable(tinybasic
    SOURCES main.c
    LIBRARIES cpt32_display cpt32_irqc
    OPTIMIZATION 0
    ORIGIN 0
    STACK_TOP 0x00100000
    ENTRY main)
