cmake_minimum_required(VERSION 3.16)
project(hs88 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

# The harmonic channels are the 3HS816 core.
if(NOT TARGET hs816)
    set(HS816_BUILD_TESTS OFF CACHE BOOL "" FORCE)
    add_subdirectory(${CMAKE_CURRENT_SOURCE_DIR}/../3hs816 ${CMAKE_CURRENT_BINARY_DIR}/hs816)
endif()

# Integer-only like hs816 (tests/test_sources.py enforces it).
add_library(hs88 STATIC
    src/biquad.cpp
    src/engine.cpp
    src/master_effects.cpp
    src/pcm_channel.cpp
    src/registers.cpp
)
target_include_directories(hs88 PUBLIC include PRIVATE src)
target_link_libraries(hs88 PUBLIC hs816)
if(MSVC)
    target_compile_options(hs88 PRIVATE /W4)
else()
    target_compile_options(hs88 PRIVATE -Wall -Wextra)
endif()

option(HS88_BUILD_TESTS "Build the 3HS88PWN4 tests" ON)
# Path to Furnace's src/engine/platform/sound/s3hs88pwn4/sound.cpp. When set,
# hs88_legacy_compare measures the engine against the float original.
set(HS816_LEGACY_SOURCE "" CACHE FILEPATH "3HS88PWN4 reference sound.cpp")

if(HS88_BUILD_TESTS)
    enable_testing()
    add_executable(hs88_tests tests/test_hs88.cpp)
    target_link_libraries(hs88_tests PRIVATE hs88)
    add_test(NAME hs88_tests COMMAND hs88_tests)

    find_package(Python3 COMPONENTS Interpreter)
    if(Python3_FOUND)
        add_test(NAME hs88_sources
                 COMMAND Python3::Interpreter -m unittest discover -s ${CMAKE_CURRENT_SOURCE_DIR}/tests -p "test_*.py")
    endif()

    if(HS816_LEGACY_SOURCE)
        add_executable(hs88_legacy_compare tests/legacy_compare.cpp)
        target_link_libraries(hs88_legacy_compare PRIVATE hs88)
        target_compile_definitions(hs88_legacy_compare PRIVATE HS816_LEGACY_SOURCE="${HS816_LEGACY_SOURCE}")
        add_test(NAME hs88_legacy_compare COMMAND hs88_legacy_compare)
    endif()
endif()
