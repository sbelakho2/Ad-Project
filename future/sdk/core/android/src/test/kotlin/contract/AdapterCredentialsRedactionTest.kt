package contract

import com.rivalapexmediation.sdk.contract.AdapterCredentials
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class AdapterCredentialsRedactionTest {
    @Test
    fun `toString hashes sensitive fields`() {
        val creds = AdapterCredentials(
            key = "my-key-123",
            secret = "super-secret",
            appId = "app-xyz",
            accountIds = mapOf("acct" to "account-secret")
        )

        val rendered = creds.toString()

        assertFalse(rendered.contains("my-key-123"))
        assertFalse(rendered.contains("super-secret"))
        assertFalse(rendered.contains("account-secret"))
        assertTrue(rendered.contains("***"))
    }
}
