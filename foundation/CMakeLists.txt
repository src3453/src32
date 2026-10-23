cmake_minimum_required(VERSION 3.20)
project(foundation LANGUAGES NONE)
add_subdirectory("${CMAKE_CURRENT_LIST_DIR}/../sdk/cpt32" cpt32)

# Kernel layout of doc/spec_Foundation.md §5.1: image at 0x10000, kernel
# stack top 0x100000 (the trap stub runs kernel_trap from --stack-top).
src32_add_library(foundation_kernel
    SOURCES kernel/sched.c kernel/proc.c kernel/mm.c kernel/trap.c kernel/ipc.c kernel/fb.c kernel/sgu.c kernel/blk.c kernel/wax.c
    # The phase 1 test reads the slots as physical memory, so it needs the slot
    # backend (doc/foundation_vm_plan.md); the demo runs on either.
    DEFINITIONS FND_MM_SLOT_ONLY
    INCLUDE_DIRECTORIES include "${CMAKE_CURRENT_LIST_DIR}/../sdk/cpt32/include")

# user_code_begin and user_code_end, the first and last functions of each
# user.c, bound the user code region that processes may execute.
src32_add_executable(foundation_demo
    SOURCES demo/user.c demo/main.c
    LIBRARIES foundation_kernel cpt32_display
    ORIGIN 0x10000
    STACK_TOP 0x100000
    OPTIMIZATION 1)

src32_add_executable(foundation_phase1_test
    SOURCES tests/phase1/user.c tests/phase1/main.c
    LIBRARIES foundation_kernel
    ORIGIN 0x10000
    STACK_TOP 0x100000
    OPTIMIZATION 1)
