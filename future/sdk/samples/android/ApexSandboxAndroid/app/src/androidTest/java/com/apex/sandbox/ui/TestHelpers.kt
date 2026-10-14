package com.apex.sandbox.ui

import androidx.test.espresso.Espresso.onData
import androidx.test.espresso.Espresso.onView
import androidx.test.espresso.action.ViewActions.click
import androidx.test.espresso.matcher.ViewMatchers.withId
import com.apex.sandbox.R
import org.hamcrest.CoreMatchers.anything

object TestHelpers {
    fun tapInit() {
        onView(withId(R.id.initBtn)).perform(click())
        Thread.sleep(400)
    }

    fun selectNetwork(position: Int) {
        onView(withId(R.id.networkSpinner)).perform(click())
        onData(anything()).atPosition(position).perform(click())
    }

    fun tapLoadInterstitialA() {
        onView(withId(R.id.loadInterstitialBtn)).perform(click())
    }

    fun tapShowInterstitialA() {
        onView(withId(R.id.showInterstitialBtn)).perform(click())
    }

    fun tapLoadRewardedA() {
        onView(withId(R.id.loadRewardedBtn)).perform(click())
    }

    fun tapShowRewardedA() {
        onView(withId(R.id.showRewardedBtn)).perform(click())
    }
}
