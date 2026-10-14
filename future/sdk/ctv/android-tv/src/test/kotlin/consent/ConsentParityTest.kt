package consent

import com.rivalapexmediation.ctv.consent.ConsentData
import com.rivalapexmediation.ctv.network.buildConsentMap
import org.junit.Assert.*
import org.junit.Test

class ConsentParityTest {
    @Test
    fun `buildConsentMap includes all flags when provided`() {
        val cd = ConsentData(
            gdprApplies = true,
            tcfString = "COabcd123...",
            usPrivacy = "1YNN",
            coppa = true
        )

        val map = buildConsentMap(cd)!!
        assertEquals(1, map["gdpr"]) // encoded as 1/0 per server convention
        assertEquals("COabcd123...", map["gdpr_consent"])
        assertEquals("1YNN", map["us_privacy"])
        assertEquals(true, map["coppa"])
    }

    @Test
    fun `buildConsentMap omits null or empty fields`() {
        val cd = ConsentData(
            gdprApplies = null,
            tcfString = "",
            usPrivacy = null,
            coppa = null
        )
        val map = buildConsentMap(cd)!!
        assertFalse(map.containsKey("gdpr"))
        assertFalse(map.containsKey("gdpr_consent"))
        assertFalse(map.containsKey("us_privacy"))
        assertFalse(map.containsKey("coppa"))
    }
}
