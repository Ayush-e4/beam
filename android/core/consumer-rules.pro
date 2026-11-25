# JNA finds native functions and struct fields by reflection, so neither JNA nor the
# UniFFI bindings (app.beam.core) may be renamed or stripped.
-keep class com.sun.jna.** { *; }
-keep class * extends com.sun.jna.** { *; }
-keep class app.beam.core.** { *; }
# JNA references desktop AWT classes that Android does not have; those paths are never used.
-dontwarn java.awt.**
