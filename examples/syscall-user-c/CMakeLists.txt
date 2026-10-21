cmake_minimum_required(VERSION 3.20)
project(cpt32_syscall_user_c LANGUAGES NONE)
add_subdirectory("${CMAKE_CURRENT_LIST_DIR}/../../sdk/cpt32" cpt32)

# user.c first is not required; user_main must precede user_code_end inside it.
src32_add_executable(syscall_user
    SOURCES user.c kernel.c
    LIBRARIES cpt32_display
    OPTIMIZATION 1)
