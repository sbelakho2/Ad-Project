# Keep public API and internal classes for unit tests to prevent R8 stripping
-keep class com.rivalapexmediation.ctv.** { *; }
-dontobfuscate
