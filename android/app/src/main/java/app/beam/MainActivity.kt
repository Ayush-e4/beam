package app.beam

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import app.beam.ui.HomeScreen
import app.beam.ui.theme.BeamTheme

/** Single activity hosting the Compose UI. */
class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent {
            BeamTheme {
                HomeScreen()
            }
        }
    }
}
