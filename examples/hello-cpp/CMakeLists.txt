cmake_minimum_required(VERSION 3.20)
project(cpt32_hello_cpp LANGUAGES NONE)
add_subdirectory("${CMAKE_CURRENT_LIST_DIR}/../../sdk/cpt32" cpt32)

src32_add_executable(hello_cpp
    SOURCES main.cpp counter.cpp
    LIBRARIES cpt32_display
    OPTIMIZATION 1)
