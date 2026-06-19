# Keep the service entry points that Android instantiates by name from the
# manifest. R8 must not rename or strip them.
-keep class com.skycua.phonecompanion.SkyAccessibilityService { *; }
-keep class com.skycua.phonecompanion.SkyNotificationListenerService { *; }
-keep class com.skycua.phonecompanion.SetupActivity { *; }
-keep class com.skycua.phonecompanion.MainActivity { *; }
