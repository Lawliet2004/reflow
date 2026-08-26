package com.reflow.android

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import com.reflow.android.ui.ReflowApp

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val pairUri = intent?.data?.toString()
        setContent { ReflowApp(initialPairUri = pairUri) }
    }
}
