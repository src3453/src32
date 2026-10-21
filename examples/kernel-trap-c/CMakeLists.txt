cmake_minimum_required(VERSION 3.20)
project(cpt32_kernel_trap_c LANGUAGES NONE)
add_subdirectory("${CMAKE_CURRENT_LIST_DIR}/../../sdk/cpt32" cpt32)

# user.c keeps user_code_begin first and user_code_end last; the kernel
# checks the order at runtime.
src32_add_executable(kernel_trap
    SOURCES user.c kernel.c
    INCLUDE_DIRECTORIES "${CMAKE_CURRENT_LIST_DIR}/../../sdk/cpt32/include"
    OPTIMIZATION 1)
