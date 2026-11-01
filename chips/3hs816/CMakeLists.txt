cmake_minimum_required(VERSION 3.16)
project(hs816 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

# The engine is integer-only (it is the model for an RTL implementation):
# no float/double in include/ or src/. tests/test_sources.py enforces it.
add_library(hs816 STATIC
    src/channel.cpp
    src/engine.cpp
    src/envelope.cpp
    src/registers.cpp
    src/wavetables.cpp
)
target_include_directories(hs816 PUBLIC include PRIVATE src)
if(MSVC)
    target_compile_options(hs816 PRIVATE /W4)
else()
    target_compile_options(hs816 PRIVATE -Wall -Wextra)
endif()

option(HS816_BUILD_TESTS "Build the 3HS816 tests" ON)
# Path to Furnace's src/engine/platform/sound/s3hs88pwn4/sound.cpp. When set,
# hs816_legacy_compare measures the engine against the 3HS88PWN4 original.
set(HS816_LEGACY_SOURCE "" CACHE FILEPATH "3HS88PWN4 reference sound.cpp")

if(HS816_BUILD_TESTS)
    enable_testing()
    add_executable(hs816_tests tests/test_hs816.cpp)
    target_link_libraries(hs816_tests PRIVATE hs816)
    add_test(NAME hs816_tests COMMAND hs816_tests)

    find_package(Python3 COMPONENTS Interpreter)
    if(Python3_FOUND)
        add_test(NAME hs816_sources
                 COMMAND Python3::Interpreter -m unittest discover -s ${CMAKE_CURRENT_SOURCE_DIR}/tests -p "test_*.py")
    endif()

    if(HS816_LEGACY_SOURCE)
        add_executable(hs816_legacy_compare tests/legacy_compare.cpp)
        target_link_libraries(hs816_legacy_compare PRIVATE hs816)
        target_compile_definitions(hs816_legacy_compare PRIVATE HS816_LEGACY_SOURCE="${HS816_LEGACY_SOURCE}")
        add_test(NAME hs816_legacy_compare COMMAND hs816_legacy_compare)
    endif()
endif()
