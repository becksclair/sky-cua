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

find_package(Qt6 6.10.0 REQUIRED COMPONENTS Core Gui Quick Widgets DBus)
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

set(KDE_INSTALL_PLUGINDIR "lib/qt6/plugins" CACHE PATH "Qt plugin install directory")
set(KDE_INSTALL_DATADIR "share" CACHE PATH "XDG data install directory")

set(SKY_CUA_CURSOR_ASSET "" CACHE FILEPATH "Path to the sky-cua cursor-chat.png asset")
if(NOT SKY_CUA_CURSOR_ASSET OR NOT EXISTS "${SKY_CUA_CURSOR_ASSET}")
    message(FATAL_ERROR "Set SKY_CUA_CURSOR_ASSET to crates/sky-cua-overlay-host/assets/cursor-chat.png")
endif()

kcoreaddons_add_plugin(sky-cua-agent-cursor
    SOURCES
        main.cpp
        agentcursoreffect.cpp
        systemcursoradapter.cpp
    INSTALL_NAMESPACE "kwin/effects/plugins"
)

target_compile_features(sky-cua-agent-cursor PRIVATE cxx_std_20)
target_include_directories(sky-cua-agent-cursor PRIVATE "${KWIN_INCLUDE_DIR}")
set_target_properties(sky-cua-agent-cursor PROPERTIES PREFIX "")
target_link_libraries(sky-cua-agent-cursor
    PRIVATE
        "${KWIN_LIBRARY}"
        Qt6::Core
        Qt6::DBus
        Qt6::Gui
        Qt6::Quick
        Qt6::Widgets
        KF6::CoreAddons
        KF6::ConfigCore
        KF6::WindowSystem
        PkgConfig::EPOXY
        PkgConfig::LIBDRM
        PkgConfig::WAYLAND_SERVER
)

install(DIRECTORY qml DESTINATION ${KDE_INSTALL_DATADIR}/kwin/effects/sky-cua-agent-cursor)
install(DIRECTORY qml DESTINATION ${KDE_INSTALL_DATADIR}/kwin-wayland/effects/sky-cua-agent-cursor)
install(
    FILES "${SKY_CUA_CURSOR_ASSET}"
    DESTINATION ${KDE_INSTALL_DATADIR}/kwin/effects/sky-cua-agent-cursor/assets
    RENAME cursor-chat.png
)
install(
    FILES "${SKY_CUA_CURSOR_ASSET}"
    DESTINATION ${KDE_INSTALL_DATADIR}/kwin-wayland/effects/sky-cua-agent-cursor/assets
    RENAME cursor-chat.png
)
