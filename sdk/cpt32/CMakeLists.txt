cmake_minimum_required(VERSION 3.20)
include("${CMAKE_CURRENT_LIST_DIR}/../../tools/llvm-src32/cmake/SRC32.cmake")

src32_add_library(cpt32_display
    SOURCES src/vpu.c src/pcg.c src/print.c src/pec_rng.c src/colors.c
    INCLUDE_DIRECTORIES include)
src32_add_library(cpt32_irqc SOURCES src/irqc.c INCLUDE_DIRECTORIES include)
src32_add_library(cpt32_storage
    SOURCES src/idc.c src/dsfs.c src/wax.c
    INCLUDE_DIRECTORIES include
    LIBRARIES cpt32_irqc)
src32_add_library(cpt32_music SOURCES src/sgub.c INCLUDE_DIRECTORIES include)
src32_add_library(cpt32 LIBRARIES cpt32_display cpt32_irqc cpt32_storage cpt32_music)
