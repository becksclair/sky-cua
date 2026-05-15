from __future__ import annotations


def adjusted_origin_for_visible_monitor(
    origin_x: int,
    origin_y: int,
    allocation_width: int,
    allocation_height: int,
    monitor_width: int,
    monitor_height: int,
) -> tuple[int, int]:
    adjusted_x = origin_x
    adjusted_y = origin_y
    if monitor_width > 0 and allocation_width > monitor_width:
        adjusted_x += round((monitor_width - allocation_width) / 2)
    if monitor_height > 0 and allocation_height > monitor_height:
        adjusted_y += round((monitor_height - allocation_height) / 2)
    return (adjusted_x, adjusted_y)
