cmake_minimum_required(VERSION 3.24)

project(sky_cua_agent_cursor_kwin_effect VERSION 0.1.0 LANGUAGES CXX)

set(CMAKE_AUTOMOC ON)
set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_LIBRARY_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}/lib")
set(BUILD_SHARED_LIBS ON)
list(APPEND CMAKE_AUTOMOC_MACRO_NAMES
    KWIN_EFFECT_FACTORY
    KWIN_EFFECT_FACTORY_SUPPORTED
    KWIN_EFFECT_FACTORY_ENABLED
    KWIN_EFFECT_FACTORY_SUPPORTED_ENABLED
)

find_package(Qt6 6.10.0 REQUIRED COMPONENTS Core Gui Widgets DBus)
find_package(KF6CoreAddons 6.22.0 REQUIRED)
find_package(KF6Config 6.22.0 REQUIRED)
find_package(KF6WindowSystem 6.22.0 REQUIRED)
find_package(PkgConfig REQUIRED)
pkg_check_modules(EPOXY REQUIRED IMPORTED_TARGET epoxy)
pkg_check_modules(LIBDRM REQUIRED IMPORTED_TARGET libdrm)
pkg_check_modules(WAYLAND_SERVER REQUIRED IMPORTED_TARGET wayland-server)

include("${KF6CoreAddons_DIR}/KF6CoreAddonsMacros.cmake")

find_library(KWIN_LIBRARY NAMES kwin REQUIRED)
set(KWIN_INCLUDE_DIR "/usr/include/kwin" CACHE PATH "Path to KWin public headers")
if(NOT EXISTS "${KWIN_INCLUDE_DIR}/effect/effect.h")
    message(FATAL_ERROR "KWIN_INCLUDE_DIR does not contain effect/effect.h")
endif()
file(READ "${KWIN_INCLUDE_DIR}/effect/effect.h" KWIN_EFFECT_HEADER)
string(FIND "${KWIN_EFFECT_HEADER}" "std::chrono::milliseconds presentTime" KWIN_PREPAINT_PRESENT_TIME_OFFSET)
string(FIND "${KWIN_EFFECT_HEADER}" "virtual void pointerMotion(PointerMotionEvent *event)" KWIN_POINTER_MOTION_OFFSET)

set(KDE_INSTALL_PLUGINDIR "lib/qt6/plugins" CACHE PATH "Qt plugin install directory")
set(KDE_INSTALL_DATADIR "share" CACHE PATH "XDG data install directory")

set(SKY_CUA_EFFECT_BUILD_ID "unstamped" CACHE STRING
    "Build identity stamp reported by the effect's BuildId() DBus slot")
set(SKY_CUA_EFFECT_ID "sky-cua-agent-cursor" CACHE STRING
    "KWin effect plugin id and output name")
if(NOT SKY_CUA_EFFECT_ID MATCHES "^sky-cua-agent-cursor(-[0-9][0-9][0-9][0-9][0-9][0-9])?$")
    message(FATAL_ERROR "SKY_CUA_EFFECT_ID must be sky-cua-agent-cursor or sky-cua-agent-cursor-NNNNNN")
endif()

configure_file(
    "${CMAKE_CURRENT_SOURCE_DIR}/metadata.json.in"
    "${CMAKE_CURRENT_BINARY_DIR}/metadata.json"
    @ONLY
)

kcoreaddons_add_plugin("${SKY_CUA_EFFECT_ID}"
    SOURCES
        main.cpp
        agentcursoreffect.cpp
        systemcursoradapter.cpp
    INSTALL_NAMESPACE "kwin/effects/plugins"
)

target_compile_features("${SKY_CUA_EFFECT_ID}" PRIVATE cxx_std_20)
target_compile_definitions("${SKY_CUA_EFFECT_ID}" PRIVATE
    SKY_CUA_EFFECT_BUILD_ID="${SKY_CUA_EFFECT_BUILD_ID}"
    SKY_CUA_EFFECT_METADATA_FILE="${CMAKE_CURRENT_BINARY_DIR}/metadata.json")
if(NOT KWIN_PREPAINT_PRESENT_TIME_OFFSET EQUAL -1)
    target_compile_definitions("${SKY_CUA_EFFECT_ID}" PRIVATE SKY_CUA_KWIN_PREPAINT_HAS_PRESENT_TIME=1)
endif()
if(NOT KWIN_POINTER_MOTION_OFFSET EQUAL -1)
    target_compile_definitions("${SKY_CUA_EFFECT_ID}" PRIVATE SKY_CUA_KWIN_EFFECT_HAS_POINTER_MOTION=1)
endif()
target_include_directories("${SKY_CUA_EFFECT_ID}" PRIVATE "${KWIN_INCLUDE_DIR}")
set_target_properties("${SKY_CUA_EFFECT_ID}" PROPERTIES
    OUTPUT_NAME "${SKY_CUA_EFFECT_ID}"
    PREFIX ""
)
target_link_libraries("${SKY_CUA_EFFECT_ID}"
    PRIVATE
        "${KWIN_LIBRARY}"
        Qt6::Core
        Qt6::DBus
        Qt6::Gui
        Qt6::Widgets
        KF6::CoreAddons
        KF6::ConfigCore
        KF6::WindowSystem
        PkgConfig::EPOXY
        PkgConfig::LIBDRM
        PkgConfig::WAYLAND_SERVER
)
